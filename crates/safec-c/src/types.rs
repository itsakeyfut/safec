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
use std::collections::{HashMap, HashSet};

use crate::ast::{
    Ast, BinOp, Declaration, EnumerationId, EnumeratorId, Expr, ExprId, ForStart, InitDeclarator,
    Initializer, Item, Parameters, Stmt, StmtId, Storage, Type, TypeId, UnOp, VaOp, spell_type,
};
use crate::diagnostics::{Code, Diagnostic, DiagnosticSink, Label};
use crate::initializer::{Fault, Shapes, initializes_whole};
use crate::sema::Resolution;
use safec_ir::ir::{Rank, Sign};
use safec_ir::source::{SourceMap, Span};
use safec_ir::target::Target;

/// A spelling that is not a constant C allows, C17 6.4.4.1 p1.
///
/// `SC01xx` and not `SC03xx` although this stage is the one that emits it:
/// `docs/diagnostics.md` hands out a code by the topic of the problem rather
/// than by the pass that finds it, and what is wrong with `09` is the shape of
/// the token.
const MALFORMED_CONSTANT: Code = Code::new("SC0106");

/// A constant this compiler has no type for, C17 6.4.4 p2.
///
/// One code for two messages, because they are one rule met two ways: a
/// constant has to have a type that holds it. A value no type in its 6.4.4.1
/// p5 list holds is the program's fault, though `clang` takes an unsuffixed
/// decimal from 2^63 to 2^64 - 1 as an extension and refuses it only under
/// `-pedantic-errors`; a floating constant is this compiler's gap, because it
/// has no floating type to give one. Same arrangement as `MISMATCH` below, for the same reason.
/// The spelling that is no constant at all is the other code.
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
/// `for`, and 6.5.15 p3 for the pair of a conditional's arms. And an operand
/// of the builtins `<stdarg.h>` is spelled over that 7.16.1 makes undefined
/// where it is written: a list that is no lvalue of `__builtin_va_list`, a
/// `__builtin_va_start` naming anything but the last parameter of the
/// variadic function it is in, and a `__builtin_va_arg` of a type the default
/// argument promotions change (#1214).
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
/// A parameter of a function definition with no name, C17 6.9.1 p5, is one
/// too: the declarator of a definition C forbids, whose remedy is in the
/// declarator.
///
/// Not `OPERANDS`, which is an operator's: a declarator is not an operator,
/// and a code is never reassigned. 6.7.6.3 p1, a function returning an array
/// or a function, is the same kind of fault and would go here.
const DECLARATOR: Code = Code::new("SC0309");

/// An initializer C17 6.7.9 does not allow where it is written. Something p3
/// says cannot be initialized at all: a function, an object of incomplete
/// type, or a variable length array, since only an array of unknown size or
/// a complete object type that is not a variable length array may have an
/// initializer. Or an array initialized by an expression, where p16 asks for
/// a brace-enclosed list; p16 is Semantics rather than a constraint, and is
/// refused for the reason an incomplete object's 6.7 p7 is, under
/// [`INCOMPLETE_OBJECT`].
///
/// Not `MISMATCH`, because no value of any type would make `int f(void) = 1;`
/// right: what has to change is the `=` or the declaration. Not `DECLARATOR`,
/// because `int f(void);` is a correct declarator, and the fault is
/// initializing it.
const INITIALIZED: Code = Code::new("SC0310");

/// An expression C requires to be a constant that is not one: so far, the
/// initializer of an object at file scope, which C17 6.7.9 p4 asks to be a
/// constant expression, and which 6.6 p7 lets be an arithmetic constant, a
/// null pointer constant or an address constant.
///
/// Not `INITIALIZED`, which is a declaration that may not be initialized at
/// all, and not `MISMATCH`, which is a value of the wrong type: here the
/// declaration and the `=` are right, and what is wrong is the shape of
/// what follows. The wording follows `clang`'s `initializer element is not a
/// compile-time constant`.
const NOT_CONSTANT: Code = Code::new("SC0317");

/// `ty`, `const`-qualified, and an array's elements with it: C17 6.7.3 p10
/// qualifies the element type of an array whose type is qualified, so an
/// element of a `const` structure's array member is not written either.
fn qualify(ast: &mut Ast, ty: TypeId) -> TypeId {
    if let Type::Array {
        element,
        length,
        written,
    } = ast.ty(ty).clone()
    {
        let element = qualify(ast, element);
        let array = ast.push_type(Type::Array {
            element,
            length,
            written,
        });
        return ast.push_const(array);
    }
    ast.push_const(ty)
}

/// A `switch` given two labels for one value: a `case` value twice, or a
/// second `default`, C17 6.8.4.2 p3.
///
/// Not `REDEFINED`, which is a name given two definitions: a `case` value is
/// no name. The headlines are clang's.
const SWITCH_LABELS: Code = Code::new("SC0321");

/// Something a `for` declares that C17 6.8.5 p3 says it may not: anything
/// but an object with storage class `auto` or `register`. With no storage
/// class specifier in the language yet, that is a function.
///
/// Not `DECLARATOR`, for the reason [`INITIALIZED`] gives: `int f(void);` is
/// a correct declarator, and what is wrong is where it is written. A storage
/// class specifier, when one is read, is the other half of p3 and comes here.
const FOR_DECLARATION: Code = Code::new("SC0311");

/// Valid C this compiler reads and cannot check yet: a member access whose
/// base this stage left with no type, as `(p - q).a`'s was until #638 gave
/// a difference `ptrdiff_t`. No base measured is untyped now (#1298).
///
/// The lowering's code, `SC0304`, rather than one of this stage's own,
/// because it is the same class of refusal: what a reader searches for is
/// what this compiler cannot do yet. A struct object is typed here and
/// refused by the lowering, whose IR holds no struct, under the same code.
const NOT_YET: Code = Code::new("SC0304");

/// Two declarations of one name whose types are not compatible, C17 6.7 p4:
/// the code the name resolution gives the same fault, where what differs is
/// an array's length, which only this stage has the value of.
const CONFLICTING: Code = Code::new("SC0313");

/// An object whose type is a struct that is not complete where C needs it
/// to be: at the end of its declarator for one with no linkage (C17 6.7 p7),
/// and by the end of the translation unit for one at file scope (6.9.2 p2,
/// which makes it a definition as if initialized with zero). A parameter of
/// a definition (6.7.6.3 p4), what a definition returns (6.9.1 p3) and what
/// a call returns (6.5.2.2 p1) are held to the same, each where it is
/// written. An object or a definition's parameter of type `void` is too,
/// since `void` is incomplete and can never be completed (6.2.5 p19), and
/// so is an array of unknown size in a block with no initializer.
///
/// Its own code: the type is a correct one and the declarator a correct
/// declarator, and what is wrong is that the struct is not complete there.
/// The wording follows `clang`'s `variable has incomplete type 'struct S'`.
/// A member access on a struct not complete where it is written is the same
/// question again (C17 6.5.2.3 p1 with 6.7.2.1 p8): there are no members to
/// name yet. So is an object of one whose value is read (6.3.2.1 p2) or
/// which is assigned to (6.5.16 p2), each where it is written.
const INCOMPLETE_OBJECT: Code = Code::new("SC0315");

/// A member access naming a member its struct does not have, C17 6.5.2.3 p1
/// ("shall name a member of that type").
///
/// Not `SC0301`, the resolver's undeclared name: that is a name looked up in
/// the scopes of ordinary identifiers, and a member is looked up in its
/// struct, a name space of its own (6.2.3), which only the type checker can
/// do because only it knows the base's type. The wording follows `clang`'s
/// `no member named 'b' in 'struct S'`.
const NO_MEMBER: Code = Code::new("SC0316");

/// A member whose type or width breaks C17 6.7.2.1: here, a bit-field's
/// width or type (#1206). The same code as `sema.rs`'s `INCOMPLETE_MEMBER`,
/// on purpose: both are a constraint on the member itself, and a width can
/// only be judged once it is a constant, which is this stage's to work out,
/// so the one code is answered in two places.
const UNFIT_MEMBER: Code = Code::new("SC0314");

/// An operand an operator writes to, or takes the address of, that cannot be
/// written to or have its address taken: the left operand of an assignment
/// and the operand of `++` or `--`, which want a modifiable lvalue (C17
/// 6.5.16 p2, 6.5.2.4 p1, 6.5.3.1 p1), not a value, a function or an array;
/// and the operand of `&`, which wants an object or a function (6.5.3.2 p1),
/// not a value.
///
/// Not `OPERANDS`, an operand of a type its operator does not take. The fix
/// for that is a different value; the fix for this is a different operand,
/// an object or one element of an array, and a code is the handle a reader
/// searches with. Not worded as `clang`'s "expression is not assignable",
/// which names neither what the operand is nor, under `&`, what was wanted.
const UNWRITABLE: Code = Code::new("SC0318");

/// An enumeration constant whose value `int` does not hold, C17 6.7.2.2 p2:
/// `enum { A = 2147483647, B };` gives `B` one more than `int` holds. The
/// wording is close to clang's, which names "the underlying type 'int'".
const ENUMERATOR_RANGE: Code = Code::new("SC0322");

/// The type of every expression in one translation unit.
#[derive(Clone, Debug)]
pub struct Types {
    of: Vec<Option<TypeId>>,
    value: Vec<Option<i128>>,
    member: Vec<Option<Member>>,
    operated: Vec<Option<TypeId>>,
    controlling: HashMap<ExprId, TypeId>,
}

/// The member a member access names: the struct definition its base's tag
/// names, and the member's position among that definition's members.
///
/// Self-contained, so that a stage reading it needs neither the resolution
/// nor a second lookup by spelling: `ast.ty(definition)` is a struct with
/// members, `path[0]` is in range of them, and each later step is in range
/// of the members of the anonymous struct or union the step before names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The struct type whose `{ ... }` declares the member, or declares the
    /// anonymous member that holds it.
    pub definition: TypeId,
    /// Which of its members, counting from zero in the order written, and
    /// on through each anonymous member it is reached through (#1207): a
    /// member of an anonymous member is the containing struct's (C17 6.7.2.1
    /// p13), so `s.a` with `a` in an anonymous union at index 1 is `[1, 0]`.
    /// Never empty.
    pub path: Vec<usize>,
}

impl Types {
    /// The type the operation `id` is performed at, where that is not the
    /// type of its result: the common type C17 6.3.1.8 gives the operands of
    /// a comparison, whose result is an `int`, and the type a compound
    /// assignment or an increment computes at before the result is
    /// converted back to its left operand's (6.5.16.2 p3), and a `switch`'s
    /// controlling expression promoted (6.8.4.2 p5). `None` for every other
    /// expression, which is performed at its own type.
    pub fn operated(&self, id: ExprId) -> Option<TypeId> {
        self.operated[id.index()]
    }

    /// The type a `switch` whose controlling expression is `condition`
    /// compares at: that expression promoted (C17 6.8.4.2 p5), to which each
    /// `case` value is converted.
    pub fn controlling(&self, condition: ExprId) -> Option<TypeId> {
        self.controlling.get(&condition).copied()
    }

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

    /// The member the member access `id` names, or `None` where `id` is not
    /// one or this stage gave it no type.
    ///
    /// # Panics
    ///
    /// If `id` came from a different [`Ast`].
    pub fn member(&self, id: ExprId) -> Option<&Member> {
        self.member[id.index()].as_ref()
    }
}

/// Give every expression a type, and report the constraints this stage checks.
///
/// Takes `&mut Ast` because two of the types it works out are types nobody
/// wrote: the `int` of an integer constant, and the pointer that `&x` has.
/// They go in the same arena as the rest, so that there is one representation
/// of a type in this compiler rather than two.
///
/// Takes the [`Target`], because what an integer constant's type is (C17
/// 6.4.4.1 p5) and what type the usual arithmetic conversions give two
/// operands (6.3.1.8) turn on every integer type's width there. See ADR-0119,
/// which replaces what ADR-0013 said this stage is given. A diagnostic that
/// turns on a width is a divergence from `docs/architecture.md`'s output
/// table, recorded there.
pub fn check(
    sources: &SourceMap,
    ast: &mut Ast,
    resolution: &Resolution,
    target: Target,
    diagnostics: &mut DiagnosticSink,
) -> Types {
    let mut checker = Checker {
        sources,
        resolution,
        types: vec![None; ast.expr_ids().count()],
        values: vec![None; ast.expr_ids().count()],
        members: vec![None; ast.expr_ids().count()],
        undefined: vec![false; ast.expr_ids().count()],
        reported_within: vec![false; ast.expr_ids().count()],
        // Pushed once. A new node per constant would fill the arena with
        // copies of `int` and make nothing truer.
        int: ast.push_type(Type::INT),
        target,
        integers: HashMap::new(),
        operated: vec![None; ast.expr_ids().count()],
        enumerator_values: vec![None; ast.enumerator_count()],
        enumerations_waiting: waiting(ast),
        enumerated: enumerated(ast, resolution),
        controlling: HashMap::new(),
        receivers: HashMap::new(),
        at: None,
    };

    checker.collect_receivers(ast, diagnostics);
    // Every comparison from here on is with the sign each enumerated type is
    // given, which is decided before any expression of it is typed.
    ast.settle_enumerations();
    checker.decide_enumerations(ast, None);

    for id in ast.expr_ids().collect::<Vec<_>>() {
        let errors = diagnostics.error_count();
        let ty = checker.type_of(ast, id, diagnostics);
        checker.types[id.index()] = ty;
        // An undeclared name was reported by the resolver, before this ran,
        // so the sink's count does not show it. The arena holds a child
        // before its parent, so every operand is marked before it is read.
        let unresolved = matches!(ast.expr(id), Expr::Identifier { .. })
            && checker.resolution.resolved(id).is_none();
        checker.reported_within[id.index()] = diagnostics.error_count() > errors
            || unresolved
            || subexpressions(ast.expr(id))
                .iter()
                .any(|operand| checker.reported_within[operand.index()]);
        // A literal's value is `constant`'s, set while it was typed; every
        // other expression's is worked out from its operands' here, before
        // anything that reads it as a null pointer constant is asked.
        if ty.is_some() && checker.values[id.index()].is_none() {
            checker.values[id.index()] = checker.evaluate(ast, id);
            checker.undefined[id.index()] =
                checker.values[id.index()].is_none() && checker.constant_shaped(ast, id);
        }
        checker.decide_enumerations(ast, Some(id));
        checker.check_received(ast, id, diagnostics);
    }

    // After every expression is typed, because a length's type and value are
    // what two of the three questions ask.
    checker.check_declarators(ast, diagnostics);
    checker.check_bit_fields(ast, diagnostics);
    checker.check_redeclared_lengths(ast, diagnostics);
    checker.check_complete_objects(ast, diagnostics);
    checker.check_definition_parameters(ast, diagnostics);
    checker.check_incomplete_values(ast, diagnostics);
    checker.check_initialized(ast, diagnostics);
    checker.check_static_initializers(ast, diagnostics);
    checker.check_for_declarations(ast, diagnostics);
    checker.check_switches(ast, diagnostics);
    checker.check_enumerators(ast, diagnostics);

    Types {
        of: checker.types,
        value: checker.values,
        member: checker.members,
        operated: checker.operated,
        controlling: checker.controlling,
    }
}

/// What waits on one expression's value: see `Checker::enumerations_waiting`.
#[derive(Debug, Default)]
struct Waiting {
    /// The constants whose values count from it, in order.
    constants: Vec<EnumeratorId>,
    /// The enumerations whose last constant is among them.
    complete: Vec<EnumerationId>,
}

/// Each written value, or `None` for the start of a list, with the constants
/// that count from it and the lists it finishes.
fn waiting(ast: &Ast) -> HashMap<Option<ExprId>, Waiting> {
    let mut waiting: HashMap<Option<ExprId>, Waiting> = HashMap::new();
    let mut from: HashMap<EnumeratorId, Option<ExprId>> = HashMap::new();
    for id in ast.enumerator_ids() {
        let enumerator = ast.enumerator(id);
        let key = match (enumerator.written, enumerator.previous) {
            (Some(written), _) => Some(written),
            (None, Some(previous)) => from[&previous],
            (None, None) => None,
        };
        from.insert(id, key);
        waiting.entry(key).or_default().constants.push(id);
    }
    for ty in ast.enumerated_types() {
        let Some(enumeration) = ast.enumeration_of(ty) else {
            continue;
        };
        let Some(Some(last)) = ast
            .enumeration(enumeration)
            .constants
            .as_ref()
            .map(|constants| constants.last().copied())
        else {
            continue;
        };
        let complete = &mut waiting.entry(from[&last]).or_default().complete;
        if !complete.contains(&enumeration) {
            complete.push(enumeration);
        }
    }
    waiting
}

/// Every enumerated type, grouped by the `enum` list whose sign it takes.
fn enumerated(ast: &Ast, resolution: &Resolution) -> HashMap<EnumerationId, Vec<TypeId>> {
    let mut grouped: HashMap<EnumerationId, Vec<TypeId>> = HashMap::new();
    for ty in ast.enumerated_types() {
        let Some(enumeration) = ast.enumeration_of(ty) else {
            continue;
        };
        let list = if ast.enumeration(enumeration).constants.is_some() {
            Some(enumeration)
        } else {
            resolution.enumeration_definition(ty)
        };
        if let Some(list) = list {
            grouped.entry(list).or_default().push(ty);
        }
    }
    grouped
}

/// The rank and sign of `ty`, where it is an integer type.
fn integer_of(ast: &Ast, ty: TypeId) -> Option<(Rank, Sign)> {
    match ast.ty(ty) {
        Type::Integer { rank, sign } => Some((*rank, *sign)),
        Type::Void
        | Type::VaList
        | Type::Pointer(_)
        | Type::Array { .. }
        | Type::Function { .. }
        | Type::Struct { .. } => None,
    }
}

/// The expressions `expr` is made of, so that a fault reported in any of
/// them is one reported within `expr`: see `Checker::reported_within`.
///
/// No `..` in any arm, so that a variant added later fails to compile here
/// until it says which of its parts are operands, rather than carrying no
/// mark and having an `int` answered above a fault already reported.
fn subexpressions(expr: &Expr) -> Vec<ExprId> {
    match expr {
        Expr::Number { span: _ }
        | Expr::Character { span: _ }
        | Expr::String {
            parts: _,
            span: _,
            length: _,
        }
        | Expr::Length { value: _, span: _ }
        | Expr::Identifier { span: _ }
        | Expr::Enumerator {
            enumerator: _,
            span: _,
        }
        | Expr::SizeofType { ty: _, span: _ }
        | Expr::Error { span: _ } => Vec::new(),
        Expr::VaOp {
            op: _,
            operands,
            span: _,
        } => operands.clone(),
        Expr::VaArg {
            list,
            ty: _,
            span: _,
        } => vec![*list],
        Expr::Unary {
            op: _,
            operand,
            span: _,
        }
        | Expr::SizeofExpr { operand, span: _ }
        | Expr::Cast {
            ty: _,
            operand,
            span: _,
        } => vec![*operand],
        Expr::Binary {
            op: _,
            lhs,
            rhs,
            span: _,
        }
        | Expr::Comma { lhs, rhs, span: _ } => vec![*lhs, *rhs],
        Expr::Assign {
            op: _,
            place,
            value,
            span: _,
        } => vec![*place, *value],
        Expr::Conditional {
            condition,
            then,
            otherwise,
            span: _,
        } => vec![*condition, *then, *otherwise],
        Expr::Call {
            callee,
            arguments,
            span: _,
        } => std::iter::once(*callee)
            .chain(arguments.iter().copied())
            .collect(),
        Expr::Subscript {
            base,
            index,
            span: _,
        } => vec![*base, *index],
        Expr::Member {
            base,
            member: _,
            arrow: _,
            span: _,
        } => vec![*base],
    }
}

/// What an expression designates, which is what C17's constraints on an
/// lvalue ask. See [`Checker::designates`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Designates {
    /// An object: an lvalue, 6.3.2.1 p1.
    Object,
    /// A function: a function designator, 6.3.2.1 p4.
    Function,
    /// A value that designates nothing: not an lvalue.
    Value,
    /// Not known, because something this depends on has no type. Whatever
    /// left it untyped was reported there, or is this compiler's gap.
    Unknown,
}

/// Where a declarator is, for the rules on an array's length: at file scope
/// and as a member of a struct the length is a constant (C17 6.7.6.2 p2,
/// 6.7.2.1 p9), and in a block or a prototype it need not be.
#[derive(Clone, Copy)]
enum Where {
    FileScope,
    Inner,
    Member,
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
    /// The controlling expression of a `switch`, C17 6.8.4.2 p1, which has
    /// to be an integer: a pointer, which a condition takes, is not one.
    Switch,
}

struct Checker<'a> {
    sources: &'a SourceMap,
    resolution: &'a Resolution,
    /// The expression being typed, so that whether a struct is complete is
    /// asked where it is (`Resolution::complete_at`) rather than where its
    /// type was written. `None` outside the walk over expressions.
    at: Option<ExprId>,
    types: Vec<Option<TypeId>>,
    /// What each integer constant is worth, filled by the same walk that
    /// fills `types` and empty everywhere else.
    values: Vec<Option<i128>>,
    /// What each member access names, filled by the same walk. See
    /// [`Types::member`].
    members: Vec<Option<Member>>,
    /// Whether each expression is a constant expression in shape whose
    /// evaluation overflows or has no defined result, so that
    /// [`Checker::evaluate`] gave it no value. Set by `check` beside the
    /// value, from [`Checker::constant_shaped`]. What tells `int a[1 << 31];`
    /// (C17 6.6 p4) from `int a[n];` (6.7.6.2 p2) when a length at file scope
    /// has no value.
    undefined: Vec<bool>,
    /// Whether a fault was reported in each expression or below it: typing
    /// it raised the sink's error count, it is a name the resolver could not
    /// resolve, or one of its [`subexpressions`] is marked. Set by `check` beside
    /// the type. What tells an expression left untyped by a report from one
    /// left untyped by this compiler's gap, which [`Checker::binary`] answers
    /// differently for an operand and [`Checker::member`] for a base.
    reported_within: Vec<bool>,
    int: TypeId,
    /// The machine this run is for, which says how wide each integer type is.
    target: Target,
    /// One type per integer type this stage has had to name itself, such as
    /// the `unsigned long` two operands convert to, so that one name is one
    /// node rather than a node per expression.
    integers: HashMap<(Rank, Sign), TypeId>,
    /// What [`Types::operated`] answers, by expression.
    operated: Vec<Option<TypeId>>,
    /// Each enumeration constant's value, by its id, once decided. See
    /// [`Checker::decide_enumerations`].
    enumerator_values: Vec<Option<i128>>,
    /// What waits on each expression's value: the constants that count from
    /// it, and the enumerations whose last value it is. `None` keys what
    /// counts from nothing written, decided before any expression is typed.
    enumerations_waiting: HashMap<Option<ExprId>, Waiting>,
    /// Every enumerated type that is each `enum` list's, by the list: the
    /// specifier itself, its `const` copies, and every `enum E` the name
    /// resolver bound to its tag. Each is given the sign the list decides.
    enumerated: HashMap<EnumerationId, Vec<TypeId>>,
    /// What [`Types::controlling`] answers, by a `switch`'s controlling
    /// expression.
    controlling: HashMap<ExprId, TypeId>,
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
        let mut declared: Vec<(Span, TypeId, Where)> = Vec::new();
        // Each struct definition's members are walked once, however many
        // declarators share it.
        let mut walked: HashSet<TypeId> = HashSet::new();
        // A declaration of a tag and nothing else has no declarator, and its
        // specifiers' type is all there is to ask: `struct S { int a[0]; };`
        // has a member to hold to the rules for arrays.
        let push = |declarators: &[InitDeclarator],
                    specified: TypeId,
                    span: Span,
                    place: Where,
                    declared: &mut Vec<(Span, TypeId, Where)>| {
            if declarators.is_empty() {
                declared.push((span, specified, place));
            }
            for declarator in declarators {
                let declaration = &declarator.declaration;
                declared.push((
                    declaration.name.unwrap_or(declaration.span),
                    declaration.written,
                    place,
                ));
            }
        };
        for item in ast.items() {
            match item {
                Item::Function(function) => {
                    declared.push((function.name, function.ty, Where::FileScope))
                }
                Item::Declaration {
                    declarators,
                    typedefs,
                    specified,
                    enumerators: _,
                    storage: _,
                    span,
                } => {
                    push(
                        declarators,
                        *specified,
                        *span,
                        Where::FileScope,
                        &mut declared,
                    );
                    // **A typedef's type is held to the same rules** (#1294):
                    // at file scope C17 6.7.6.2 p2 forbids a variably
                    // modified one, as it does an object's. Mutation: leave
                    // typedefs out; `a_variable_length_typedef_at_file_scope_is_refused`
                    // is accepted.
                    declared.extend(typedefs.iter().map(|&ty| (*span, ty, Where::FileScope)));
                }
                Item::Error { .. } => {}
            }
        }
        for id in ast.stmt_ids() {
            if let Stmt::Declaration {
                declarators,
                typedefs,
                specified,
                enumerators: _,
                storage: _,
                span,
            } = ast.stmt(id)
            {
                push(declarators, *specified, *span, Where::Inner, &mut declared);
                declared.extend(typedefs.iter().map(|&ty| (*span, ty, Where::Inner)));
            }
        }

        // Grows as it is read: every function type met on the way down hands
        // its parameters on as declarations of their own.
        let mut next = 0;
        while let Some(&(at, written, place)) = declared.get(next) {
            next += 1;
            let mut reported = false;
            let mut current = written;
            loop {
                match ast.ty(current) {
                    // A definition's members are declarations of their own,
                    // each held to the rules an array has.
                    Type::Struct { members, .. } => {
                        if let Some(members) = members {
                            if walked.insert(current) {
                                for member in members {
                                    declared.push((
                                        member.name.unwrap_or(member.span),
                                        member.written,
                                        Where::Member,
                                    ));
                                }
                            }
                        }
                        break;
                    }
                    Type::Array {
                        element,
                        length,
                        written: _,
                    } => {
                        let (element, length) = (*element, *length);
                        if !reported {
                            reported =
                                self.report_array(ast, at, element, length, place, diagnostics);
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
                                    Where::Inner,
                                ));
                            }
                        }
                        current = *returns;
                    }
                    Type::Integer {
                        rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                        sign: _,
                    }
                    | Type::Void
                    | Type::VaList => break,
                }
            }
        }
    }

    /// Report every initializer of an object at file scope, or of a `static`
    /// one in a block, that is not a constant expression, under
    /// [`NOT_CONSTANT`]: C17 6.7.9 p4 asks one of an object with static
    /// storage duration, which is those two. It may be an integer constant expression, whose
    /// value `values` holds, a null pointer constant among them, or an
    /// address constant ([`Checker::address_constant`]). One that is constant
    /// in shape and has no defined value is told so (6.6 p4), as a
    /// file-scope array length is. One this stage left untyped is declined
    /// as `NOT_YET` unless the run has already failed, since the lowering
    /// drops the initializer unread and would otherwise build the program.
    /// A function given one is `INITIALIZED`'s.
    ///
    /// A block-scope `extern` with an initializer is refused here as well,
    /// under [`INITIALIZED`]: 6.7.9 p5 forbids one to a block-scope name with
    /// linkage, and the lowering takes the object from file scope and would
    /// drop it unread.
    fn check_static_initializers(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        let at_file_scope = ast.items().iter().filter_map(|item| match item {
            Item::Declaration { declarators, .. } => Some((declarators, true)),
            Item::Function(_) | Item::Error { .. } => None,
        });
        let mut in_a_block = Vec::new();
        for id in ast.stmt_ids() {
            if let Stmt::Declaration {
                declarators,
                storage: Some(storage),
                ..
            } = ast.stmt(id)
            {
                match storage {
                    Storage::Static => in_a_block.push((declarators, false)),
                    Storage::Extern => {
                        for declarator in declarators {
                            let Some(init) = &declarator.init else {
                                continue;
                            };
                            diagnostics.report(
                                Diagnostic::error("an `extern` in a block cannot be initialized")
                                    .with_code(INITIALIZED)
                                    .with_label(Label::primary(
                                        init.span(ast),
                                        "this initializes it",
                                    ))
                                    .with_note(
                                        "C17 6.7.9 p5: a block-scope name with linkage has no initializer",
                                    ),
                            );
                        }
                    }
                }
            }
        }
        for (declarators, file_scope) in at_file_scope.chain(in_a_block) {
            // Each expression of a list is held to it as one alone is: what
            // each initializes is a constant or an address (C17 6.7.9 p4).
            let each = declarators.iter().flat_map(|declarator| {
                declarator
                    .init
                    .iter()
                    .flat_map(Initializer::exprs)
                    .map(move |init| (declarator, init))
            });
            for (declarator, init) in each {
                let declaration = &declarator.declaration;
                // A function with an initializer is `INITIALIZED`'s, and no
                // initializer would make it right.
                if matches!(ast.ty(declaration.ty), Type::Function { .. }) {
                    continue;
                }
                let name = self
                    .sources
                    .snippet(declaration.name.unwrap_or(declaration.span));
                let span = ast.expr(init).span();
                if self.types[init.index()].is_none() {
                    // Either the program's fault, reported where it was
                    // typed, or this compiler's gap, which nobody reported
                    // (`p - q` was until #638, and none is measured now,
                    // #1298): the lowering drops
                    // a file-scope initializer unread, so passing it over
                    // here would build a program C forbids. Declined unless
                    // something was already reported, since then the run
                    // fails either way.
                    if !diagnostics.has_errors() {
                        diagnostics.report(
                            Diagnostic::error(format!(
                                "cannot check whether the initializer of `{name}` is a constant"
                            ))
                            .with_code(NOT_YET)
                            .with_label(Label::primary(span, "this compiler gave this no type"))
                            .with_note(
                                "a gap in this compiler: whether this initializer is valid C is not checked",
                            ),
                        );
                    }
                    continue;
                }
                if self.values[init.index()].is_some() || self.address_constant(ast, init) {
                    continue;
                }
                // **A difference of two address constants is this
                // compiler's gap** (#638): C17 6.6 p7 lists no such form,
                // and p10 lets an implementation take others, which `clang`
                // does even under `-pedantic-errors`, folding `&x[3] -
                // &x[0]`. Saying the program is wrong about one it builds
                // would be a report about ordinary C; this compiler does not
                // fold one yet. Mutation: drop this; the test
                // `a_difference_of_two_addresses_in_a_static_is_this_compilers_gap`
                // says `g` is no constant.
                if self.holds_address_difference(ast, init) {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "cannot check whether the initializer of `{name}` is a constant"
                        ))
                        .with_code(NOT_YET)
                        .with_label(Label::primary(
                            span,
                            "a difference of two addresses, which this compiler does not fold",
                        ))
                        .with_note(
                            "a gap in this compiler: whether this initializer is valid C is not checked",
                        ),
                    );
                    continue;
                }
                let diagnostic = if self.undefined[init.index()] {
                    Diagnostic::error(format!("the initializer of `{name}` has no defined value"))
                        .with_label(Label::primary(
                            span,
                            "this overflows its type, divides by zero, or shifts too far",
                        ))
                        .with_note(
                            "a constant expression evaluates to a value its type can hold (C17 6.6 p4)",
                        )
                } else {
                    Diagnostic::error(format!("the initializer of `{name}` is not a constant"))
                        .with_label(Label::primary(span, "not a constant expression"))
                        .with_note(if file_scope {
                            "an object at file scope is initialized with a constant expression (C17 6.7.9 p4)"
                        } else {
                            "a `static` object is initialized with a constant expression (C17 6.7.9 p4)"
                        })
                };
                diagnostics.report(diagnostic.with_code(NOT_CONSTANT));
            }
        }
    }

    /// Whether `id` is an address constant, C17 6.6 p9, or one plus or minus
    /// an integer constant expression (p7).
    ///
    /// Two questions asked in turn of one walk down the expression. As an
    /// address: an expression of array or function type that designates
    /// something with static storage duration, which 6.3.2.1 p3 and p4 make
    /// a pointer; `&e` of such a designator; or either of those with an
    /// integer constant added, on either side, or subtracted on the right.
    /// As a designator of something with static storage duration: a name,
    /// every one of which a file-scope initializer can see being at file
    /// scope; `e[i]` with one side an address and the other an integer
    /// constant; `e.m` of a designator; and `e->m` and `*e` of an address.
    /// Those are the operators p9 lets an address constant be made with, so
    /// `?:`, which it does not list, makes none.
    ///
    /// A loop rather than a recursion, since every step goes to one operand
    /// and `x + 1 + 1 + ...` or `s.a.b.c...` is as long as the source makes
    /// it: the parser bounds the depth of nesting, not of a chain.
    /// Whether `id` holds, anywhere in it, a pointer minus a pointer whose
    /// operands are both address constants (C17 6.6 p9), which an
    /// implementation may take as a constant (p10). Walked with a stack, as
    /// [`Checker::address_constant`] is, so a chain as long as the source
    /// makes it does not run out of stack.
    fn holds_address_difference(&self, ast: &Ast, id: ExprId) -> bool {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            let expr = ast.expr(id);
            if let Expr::Binary {
                op: BinOp::Sub,
                lhs,
                rhs,
                ..
            } = *expr
            {
                let pointer = |id: ExprId| {
                    self.types[id.index()].is_some_and(|ty| {
                        matches!(ast.ty(ty), Type::Pointer(_) | Type::Array { .. })
                    })
                };
                if pointer(lhs)
                    && pointer(rhs)
                    && self.address_constant(ast, lhs)
                    && self.address_constant(ast, rhs)
                {
                    return true;
                }
            }
            pending.extend(subexpressions(expr));
        }
        false
    }

    fn address_constant(&self, ast: &Ast, id: ExprId) -> bool {
        let constant = |id: ExprId| self.values[id.index()].is_some();
        let (mut id, mut designator) = (id, false);
        loop {
            if designator {
                id = match *ast.expr(id) {
                    // A name, or a string literal, which has static storage
                    // duration (C17 6.4.5 p6).
                    Expr::Identifier { .. } | Expr::String { .. } => return true,
                    Expr::Subscript { base, index, .. } => {
                        designator = false;
                        if constant(index) {
                            base
                        } else if constant(base) {
                            index
                        } else {
                            return false;
                        }
                    }
                    Expr::Member {
                        base, arrow: false, ..
                    } => base,
                    Expr::Member {
                        base, arrow: true, ..
                    }
                    | Expr::Unary {
                        op: UnOp::Deref,
                        operand: base,
                        ..
                    } => {
                        designator = false;
                        base
                    }
                    _ => return false,
                };
                continue;
            }
            id = match *ast.expr(id) {
                Expr::Unary {
                    op: UnOp::AddrOf,
                    operand,
                    ..
                } => {
                    designator = true;
                    operand
                }
                Expr::Binary {
                    op: BinOp::Add,
                    lhs,
                    rhs,
                    ..
                } => {
                    if constant(rhs) {
                        lhs
                    } else if constant(lhs) {
                        rhs
                    } else {
                        return false;
                    }
                }
                Expr::Binary {
                    op: BinOp::Sub,
                    lhs,
                    rhs,
                    ..
                } if constant(rhs) => lhs,
                // 6.6 p9: an integer constant cast to a pointer type is an
                // address constant, and a pointer cast of one is one.
                Expr::Cast { ty, operand, .. } if matches!(ast.ty(ty), Type::Pointer(_)) => {
                    if constant(operand) {
                        return true;
                    }
                    operand
                }
                // An array or a function is a pointer once it is read, and
                // is an address constant when it designates something with
                // static storage duration.
                _ if self.types[id.index()].is_some_and(|ty| {
                    matches!(ast.ty(ty), Type::Array { .. } | Type::Function { .. })
                }) =>
                {
                    designator = true;
                    id
                }
                _ => return false,
            };
        }
    }

    /// Report a declaration of a name with linkage whose array lengths
    /// differ from an earlier one's: two array types with constant lengths
    /// are compatible only where the lengths are equal (C17 6.7.6.2 p6), and
    /// 6.7 p4 asks every declaration of one object or function to have
    /// compatible types. The name resolution compares the rest of the type
    /// and cannot compare this, having no values, so `extern int g[10]; int
    /// g[2];` gave the object its first length.
    ///
    /// **Against every earlier declaration, not the first**: `extern int
    /// a[]; int a[2]; int a[3];` has a first with no length to differ from.
    /// **And a block's `extern` declarations**, which declare the same object
    /// (6.2.2 p4). Found by review.
    ///
    /// Mutation: skip this; `an_array_redeclared_with_another_length_is_refused`
    /// is accepted. Mutation: compare with the first declaration alone; its
    /// `a` is.
    fn check_redeclared_lengths(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        // An array whose length `Parser::completed` counted from a string
        // literal rather than one the source wrote.
        let counted = |ast: &Ast, ty: TypeId| {
            matches!(
                ast.ty(ty),
                Type::Array { length: Some(length), .. }
                    if matches!(ast.expr(*length), Expr::Length { .. })
            )
        };
        let mut declared: Vec<(Span, TypeId)> = Vec::new();
        for item in ast.items() {
            match item {
                Item::Declaration { declarators, .. } => {
                    declared.extend(declarators.iter().filter_map(|declarator| {
                        Some((declarator.declaration.name?, declarator.declaration.ty))
                    }))
                }
                Item::Function(function) => declared.push((function.name, function.ty)),
                _ => {}
            }
        }
        for id in ast.stmt_ids() {
            if let Stmt::Declaration {
                declarators,
                storage: Some(Storage::Extern),
                ..
            } = ast.stmt(id)
            {
                declared.extend(declarators.iter().filter_map(|declarator| {
                    Some((declarator.declaration.name?, declarator.declaration.ty))
                }));
            }
        }
        let mut by_name: HashMap<&str, Vec<(Span, TypeId)>> = HashMap::new();
        for (name, ty) in declared {
            let spelled = self.sources.snippet(name);
            let earlier = by_name.entry(spelled).or_default();
            if let Some(&(before, before_ty)) = earlier
                .iter()
                .find(|&&(_, before_ty)| self.lengths_disagree(ast, before_ty, ty))
            {
                // `char s[5]; char s[] = "ab";` is valid: the composite type
                // is `char[5]` (C17 6.2.7 p3), which this compiler does not
                // build where the literal completed the second, so that is
                // its gap and not the conflict it is not (#1244 review).
                // Mutation: drop this arm; the case
                // `an_array_a_literal_completes_after_one_with_a_length_is_not_built_yet`
                // says `SC0313`.
                if counted(ast, ty) {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "cannot complete `{spelled}` from its initializer where an earlier \
                             declaration gave its length yet"
                        ))
                        .with_code(NOT_YET)
                        .with_label(Label::primary(name, "its length is its string literal's"))
                        .with_label(Label::secondary(before, "an earlier declaration"))
                        .with_note("a gap in this compiler: C17 6.2.7 p3 makes the two one type"),
                    );
                    earlier.push((name, ty));
                    continue;
                }
                diagnostics.report(
                    Diagnostic::error(format!("conflicting types for `{spelled}`"))
                        .with_code(CONFLICTING)
                        .with_label(Label::primary(
                            name,
                            format!("declared here as `{}`", self.spelled(ast, ty)),
                        ))
                        .with_label(Label::secondary(
                            before,
                            format!("previously declared as `{}`", self.spelled(ast, before_ty)),
                        ))
                        .with_note("C17 6.7 p4 and 6.7.6.2 p6"),
                );
            }
            earlier.push((name, ty));
        }
    }

    /// Whether `left` and `right` hold, at one place in their derivations,
    /// two arrays whose lengths are both constants and differ, or an
    /// enumerated type and an integer type it is not: through arrays,
    /// pointers, and a function's return and each parameter of two
    /// prototypes, as `Ast::compatible` walks them. A worklist, as every walk
    /// of a type here is. Mutation: leave out the function arm;
    /// `an_array_redeclared_with_another_length_is_refused`'s `f` is
    /// accepted.
    ///
    /// **Both are what the name resolver could not compare**, since it runs
    /// before any value: the second because an enumerated type's sign is
    /// decided by the values of its constants (C17 6.7.2.2 p4). So `enum E f(void);
    /// unsigned f(void);` is one function where an enumeration is unsigned and
    /// two where it is `int`. Mutation: leave out the integer arm;
    /// `an_enumerated_type_is_compatible_with_its_integer_type_alone` accepts
    /// `g`.
    fn lengths_disagree(&self, ast: &Ast, left: TypeId, right: TypeId) -> bool {
        let value = |length: &Option<ExprId>| length.and_then(|id| self.values[id.index()]);
        let mut work = vec![(left, right)];
        while let Some((left, right)) = work.pop() {
            match (ast.ty(left), ast.ty(right)) {
                (
                    Type::Array {
                        element: left_element,
                        length: left_length,
                        ..
                    },
                    Type::Array {
                        element: right_element,
                        length: right_length,
                        ..
                    },
                ) => {
                    if let (Some(l), Some(r)) = (value(left_length), value(right_length)) {
                        if l != r {
                            return true;
                        }
                    }
                    work.push((*left_element, *right_element));
                }
                (Type::Pointer(l), Type::Pointer(r)) => work.push((*l, *r)),
                (
                    Type::Integer {
                        rank: left_rank,
                        sign: left_sign,
                    },
                    Type::Integer {
                        rank: right_rank,
                        sign: right_sign,
                    },
                ) if ast.enumeration_of(left).is_some() != ast.enumeration_of(right).is_some() => {
                    if (left_rank, left_sign) != (right_rank, right_sign) {
                        return true;
                    }
                }
                (
                    Type::Function {
                        returns: left_returns,
                        parameters: left_parameters,
                        ..
                    },
                    Type::Function {
                        returns: right_returns,
                        parameters: right_parameters,
                        ..
                    },
                ) => {
                    work.push((*left_returns, *right_returns));
                    if let (Parameters::Prototype(l), Parameters::Prototype(r)) =
                        (left_parameters, right_parameters)
                    {
                        // **As adjusted**, which is what C17 6.7.6.3 p15
                        // compares: `void f(int a[2]); void f(int a[3]);`
                        // both take an `int *`, and compared as written they
                        // were refused (#1136). An inner length survives the
                        // adjustment and is still compared, against a pointer
                        // to an array too, which as written was never reached.
                        // Mutation: compare `written`;
                        // `array_parameters_of_different_lengths_declare_one_function`
                        // is refused, and `an_array_redeclared_with_another_length_is_refused`'s
                        // `d` accepted.
                        work.extend(l.iter().zip(r).map(|(l, r)| (l.ty, r.ty)));
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Hold every declarator that has an initializer to C17 6.7.9 p3, and
    /// report one that initializes what cannot be under [`INITIALIZED`].
    ///
    /// At file scope and in a block alike, since p3 is about the entity and
    /// not where it is. `assignable` answers nothing for a function, `void` or
    /// an array target, so without this `int f(void) = 1;` was accepted in
    /// silence and its initializer dropped. A variable length array is one
    /// with a length down its chain of elements that has no value, which
    /// [`Checker::is_variable_length`] asks; at file scope the same declarator
    /// is also 6.7.6.2 p2's, and both are reported, being two constraints.
    ///
    /// **A length whose evaluation overflows is answered as variable too.**
    /// `int a[1 << 31] = 1;` in a block breaks 6.6 p4 rather than p3, and
    /// nothing else refuses that length in a block, so calling it variable is
    /// the wrong reason given for a right refusal, where leaving it out would
    /// accept it in silence.
    fn check_initialized(&mut self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
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
            let Some(initializer) = &declarator.init else {
                continue;
            };
            let init_span = initializer.span(ast);
            let declaration = &declarator.declaration;
            let ty = declaration.ty;
            let name = declaration.name.unwrap_or(declaration.span);
            let called = self.sources.snippet(name);
            // Every variant named, so that a type added later is asked here.
            let (message, clause) = match ast.ty(ty) {
                Type::Function { .. } => (
                    format!("`{called}` is a function, and only an object can be initialized"),
                    "C17 6.7.9 p3",
                ),
                Type::Void => (
                    format!(
                        "`{called}` has type `void`, which is incomplete, and only a complete object can be initialized"
                    ),
                    "C17 6.7.9 p3",
                ),
                Type::Array { .. } if self.is_variable_length(ast, ty) => (
                    format!("`{called}` is a variable length array, which cannot be initialized"),
                    "C17 6.7.9 p3",
                ),
                // A braced list, for any object type: what each item
                // initializes is `Checker::check_list`'s.
                _ if initializer.expr().is_none() => {
                    self.check_list(ast, ty, name, initializer, diagnostics);
                    continue;
                }
                // p14 lets a string literal initialize an array of character
                // type, and p2 forbids one longer than the array, its
                // terminating zero aside: `char s[2] = "ab";` drops the zero.
                // An array of unknown size was given the literal's length
                // when it was declared (`Parser::completed`).
                Type::Array {
                    element, length, ..
                } if initializer
                    .expr()
                    .is_some_and(|init| matches!(ast.expr(init), Expr::String { .. })) =>
                {
                    let Some(Expr::String {
                        length: literal, ..
                    }) = initializer.expr().map(|init| ast.expr(init))
                    else {
                        unreachable!("matched as a string literal above")
                    };
                    let character = matches!(
                        ast.ty(ast.unqualified(*element)),
                        Type::Integer {
                            rank: Rank::Char,
                            ..
                        }
                    );
                    // Only a bare literal reaches here for an array of any
                    // other element: the parser keeps the braces of `char
                    // a[2][3] = { "ab" }`, which is brace elision (p20) and
                    // waits for braced lists, and `char a[2][3] = "ab"` is
                    // the p16 violation this says it is.
                    if !character {
                        (
                            format!(
                                "`{called}` is not an array of character type, and only one is \
                                 initialized from a string literal"
                            ),
                            "C17 6.7.9 p14 and p16",
                        )
                    } else {
                        let holds = length.and_then(|id| self.values[id.index()]);
                        let needs = self.values[literal.index()];
                        match (holds, needs) {
                            (Some(holds), Some(needs)) if needs - 1 > holds => (
                                format!(
                                    "the string literal has {} characters, and `{called}` holds {holds}",
                                    needs - 1
                                ),
                                "C17 6.7.9 p2: no initializer provides a value for an object \
                                 outside the one initialized",
                            ),
                            _ => continue,
                        }
                    }
                }
                // p16: "the initializer for an object that has aggregate or
                // union type shall be a brace-enclosed list of initializers",
                // and p14 allows a string literal for an array of character
                // type, which the arm above takes. A braced list is
                // `Checker::check_list`'s, so whatever initializer reaches
                // here is neither. Every kind of
                // expression is named, so that the one a compound literal
                // becomes stops the build here rather than being refused as
                // this; the string arm is kept only so the match is whole.
                Type::Array { .. }
                    if initializer.expr().is_some_and(|init| match ast.expr(init) {
                        Expr::String { .. } => false,
                        Expr::Length { .. }
                        | Expr::Number { .. }
                        | Expr::Character { .. }
                        | Expr::Identifier { .. }
                        | Expr::Enumerator { .. }
                        | Expr::Unary { .. }
                        | Expr::Binary { .. }
                        | Expr::Assign { .. }
                        | Expr::Conditional { .. }
                        | Expr::Call { .. }
                        | Expr::VaOp { .. }
                        | Expr::VaArg { .. }
                        | Expr::Subscript { .. }
                        | Expr::Member { .. }
                        | Expr::SizeofType { .. }
                        | Expr::SizeofExpr { .. }
                        | Expr::Cast { .. }
                        | Expr::Comma { .. }
                        | Expr::Error { .. } => true,
                    }) =>
                {
                    (
                        format!(
                            "`{called}` is an array, and its initializer is not a brace-enclosed list"
                        ),
                        "C17 6.7.9 p16",
                    )
                }
                Type::Array { .. } => continue,
                // Refused by `assignable`, as an assignment of one is.
                Type::VaList => continue,
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Pointer(_)
                | Type::Struct { .. } => continue,
            };
            diagnostics.report(
                Diagnostic::error(message)
                    .with_code(INITIALIZED)
                    .with_label(Label::primary(
                        name,
                        format!("declared as `{}`", self.spelled(ast, ty)),
                    ))
                    .with_label(Label::secondary(init_span, "this initializer"))
                    .with_note(clause),
            );
        }
    }

    /// Hold a braced list to the object of `ty` it initializes, declared at
    /// `name`: worked out into its leaves by `initializer::Shapes`, each
    /// held to its subobject as an initializer of it is (6.7.9 p11, by way
    /// of 6.5.16.1), and refused where it has more items than subobjects
    /// (p2), where it is `{}`, or where an array's length is not to be had
    /// from it here (`Parser::completed`).
    ///
    /// Mutation: skip the leaves; `a_braced_initializer_is_held_to_each_subobject`
    /// accepts a pointer initialized from an `int`.
    fn check_list(
        &mut self,
        ast: &Ast,
        ty: TypeId,
        name: Span,
        init: &Initializer,
        diagnostics: &mut DiagnosticSink,
    ) {
        let called = self.sources.snippet(name);
        let resolved = {
            let length = |id: ExprId| self.values[id.index()].and_then(|v| u64::try_from(v).ok());
            let whole = |value: ExprId, ty: TypeId| self.initializes_whole(ast, value, ty);
            let shapes = Shapes {
                ast,
                resolution: self.resolution,
                length: &length,
                whole: &whole,
            };
            shapes.resolve(ty, init)
        };
        let leaves = match resolved {
            Ok(leaves) => leaves,
            Err(Fault::TooMany(at)) => {
                diagnostics.report(
                    Diagnostic::error(format!("the initializer of `{called}` has too many items"))
                        .with_code(INITIALIZED)
                        .with_label(Label::primary(at, "nothing is left for this to initialize"))
                        .with_label(Label::secondary(
                            name,
                            format!("declared as `{}`", self.spelled(ast, ty)),
                        ))
                        .with_note(
                            "C17 6.7.9 p2: no initializer provides a value for an object \
                             outside the one initialized",
                        ),
                );
                return;
            }
            Err(Fault::Empty(at)) => {
                diagnostics.report(
                    Diagnostic::error("an empty initializer is not read yet")
                        .with_code(NOT_YET)
                        .with_label(Label::primary(at, "`{}`"))
                        .with_note(
                            "a gap in this compiler: C17 asks for at least one item, and C23 \
                             makes `{}` zero",
                        ),
                );
                return;
            }
            // A struct with no definition here is refused as an incomplete
            // object (`check_complete_objects`), and a second report would
            // be about the same fault.
            Err(Fault::Unsized(at)) => {
                if matches!(ast.ty(ty), Type::Array { length: None, .. }) {
                    diagnostics.report(
                        Diagnostic::error(format!(
                            "cannot take the length of `{called}` from this list yet"
                        ))
                        .with_code(NOT_YET)
                        .with_label(Label::primary(at, "this elides the braces of an element"))
                        .with_note(
                            "a gap in this compiler: C17 6.7.9 p20 and p22 give it a length, \
                             which this compiler counts only from one item per element",
                        ),
                    );
                }
                return;
            }
        };
        for leaf in leaves {
            if leaf.whole && matches!(ast.expr(leaf.value), Expr::String { .. }) {
                self.check_literal_fits(ast, leaf.ty, leaf.value, name, diagnostics);
                continue;
            }
            self.receivers
                .insert(leaf.value, Receiving::Initializer { ty: leaf.ty, name });
            self.check_received(ast, leaf.value, diagnostics);
        }
    }

    /// Whether `value` initializes an aggregate of `ty` whole rather than
    /// its first scalar: a struct of a compatible type (C17 6.7.9 p13), or
    /// a string literal for an array of character type (p14).
    fn initializes_whole(&self, ast: &Ast, value: ExprId, ty: TypeId) -> bool {
        initializes_whole(ast, self.resolution, self.types[value.index()], value, ty)
    }

    /// Refuse the string literal `literal` where it has more characters,
    /// its terminating zero aside, than the array of `ty` holds (C17 6.7.9
    /// p2 and p14).
    fn check_literal_fits(
        &self,
        ast: &Ast,
        ty: TypeId,
        literal: ExprId,
        name: Span,
        diagnostics: &mut DiagnosticSink,
    ) {
        let (Type::Array { length, .. }, Expr::String { length: needs, .. }) =
            (ast.ty(ty), ast.expr(literal))
        else {
            return;
        };
        let holds = length.and_then(|id| self.values[id.index()]);
        let needs = self.values[needs.index()];
        let (Some(holds), Some(needs)) = (holds, needs) else {
            return;
        };
        if needs - 1 > holds {
            diagnostics.report(
                Diagnostic::error(format!(
                    "the string literal has {} characters, and the array it initializes holds {holds}",
                    needs - 1
                ))
                .with_code(INITIALIZED)
                .with_label(Label::primary(ast.expr(literal).span(), "this string literal"))
                .with_label(Label::secondary(name, "in this initializer"))
                .with_note(
                    "C17 6.7.9 p2: no initializer provides a value for an object outside \
                     the one initialized",
                ),
            );
        }
    }

    /// Hold every declaration that begins a `for` to C17 6.8.5 p3, and report
    /// a declarator in one that declares a function under [`FOR_DECLARATION`].
    ///
    /// Every other constraint on such a declaration is checked where a block's
    /// is, because it is the same `Stmt::Declaration`; this one is the `for`'s
    /// own, so it is asked from the `for`.
    fn check_for_declarations(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        for id in ast.stmt_ids() {
            let Stmt::For {
                start: Some(ForStart::Declaration(declaration)),
                ..
            } = ast.stmt(id)
            else {
                continue;
            };
            // `ForStart` promises a declaration statement, and the parser is
            // the one place that builds it.
            let Stmt::Declaration {
                declarators,
                typedefs: _,
                specified,
                enumerators: _,
                storage,
                span,
            } = ast.stmt(*declaration)
            else {
                unreachable!("a `for` begins with a statement that is not a declaration")
            };
            // p3 again: an object of storage class `auto` or `register` only,
            // so neither `static` nor `extern`.
            if let Some(storage) = storage {
                let written = match storage {
                    Storage::Static => "static",
                    Storage::Extern => "extern",
                };
                diagnostics.report(
                    Diagnostic::error(format!("a `for` may not declare an object `{written}`"))
                        .with_code(FOR_DECLARATION)
                        .with_label(Label::primary(*span, format!("this is `{written}`")))
                        .with_note("C17 6.8.5 p3"),
                );
            }
            // A tag and nothing else, `for (struct S; ...)`: the identifier it
            // declares is a tag, which is not an object either.
            if declarators.is_empty() {
                diagnostics.report(
                    Diagnostic::error(format!(
                        "`{}` is a tag, and a `for` may declare only objects",
                        self.spelled(ast, *specified)
                    ))
                    .with_code(FOR_DECLARATION)
                    .with_label(Label::primary(*span, "this declares a tag"))
                    .with_note("C17 6.8.5 p3"),
                );
            }
            for declarator in declarators {
                let declaration = &declarator.declaration;
                // Every variant named, so that a type added later is asked here.
                match ast.ty(declaration.ty) {
                    Type::Function { .. } => {}
                    Type::Integer {
                        rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                        sign: _,
                    }
                    | Type::Void
                    | Type::VaList
                    | Type::Pointer(_)
                    | Type::Array { .. }
                    | Type::Struct { .. } => {
                        continue;
                    }
                }
                let name = declaration.name.unwrap_or(declaration.span);
                diagnostics.report(
                    Diagnostic::error(format!(
                        "`{}` is a function, and a `for` may declare only objects",
                        self.sources.snippet(name)
                    ))
                    .with_code(FOR_DECLARATION)
                    .with_label(Label::primary(
                        name,
                        format!("declared as `{}`", self.spelled(ast, declaration.ty)),
                    ))
                    .with_note("C17 6.8.5 p3"),
                );
            }
        }
    }

    /// Whether `ty` is complete where the expression being typed is, or where
    /// it was written outside the walk over expressions. See [`Checker::at`].
    fn complete_here(&self, ast: &Ast, ty: TypeId) -> bool {
        match self.at {
            Some(at) => self.resolution.complete_at(ast, ty, at),
            None => self.resolution.complete(ast, ty),
        }
    }

    /// Report every parameter of a function definition that has no name, under
    /// [`DECLARATOR`]: C17 6.9.1 p5, a constraint, says "the declaration of
    /// each parameter shall include an identifier", the lone `void` aside.
    ///
    /// Only the defined function's own list is asked, since p5 is about the
    /// definition's declarator: `g`'s parameters in `int f(int (*g)(int))`
    /// belong to a declarator inside it and need no names. `(void)` never
    /// arrives, because the parser makes it an empty prototype (6.7.6.3 p10).
    /// An unnamed parameter that is also incomplete keeps its `SC0315`: they
    /// are two faults, since naming it would not complete it.
    fn check_definition_parameters(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        for item in ast.items() {
            let Item::Function(function) = item else {
                continue;
            };
            let Type::Function {
                parameters: Parameters::Prototype(parameters),
                ..
            } = ast.ty(function.ty)
            else {
                continue;
            };
            for parameter in parameters.iter().filter(|p| p.name.is_none()) {
                diagnostics.report(
                    Diagnostic::error("a parameter of a function definition has no name")
                        .with_code(DECLARATOR)
                        .with_label(Label::primary(parameter.span, "this parameter"))
                        .with_note(
                            "C17 6.9.1 p5: each parameter of a definition is named, except a lone `void`; leaving one unnamed is C23",
                        ),
                );
            }
        }
    }

    /// Report every object whose type is `void`, an array of unknown size in a
    /// block with no initializer, or a struct not complete where C needs it to
    /// be, under [`INCOMPLETE_OBJECT`]: in a block, at the
    /// end of its declarator (C17 6.7 p7), which is where its type was
    /// written; at file scope, by the end of the translation unit (6.9.2 p2),
    /// which is whether it is complete where written or its tag has a
    /// definition at all. A
    /// struct with no tag has no name to define and is complete where
    /// written.
    ///
    /// A definition's parameters (6.7.6.3 p4) and what it returns (6.9.1 p3)
    /// are asked where they are written, which is before the body, so a
    /// definition of the struct inside the body completes neither. A
    /// parameter of a declaration that is not a definition may be incomplete
    /// (6.7.6.3 p12), `void` included, and a call to one is asked in `call`
    /// and in `check_argument`.
    fn check_complete_objects(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        // Each with whether it is at file scope and whether it is initialized.
        //
        // **An `extern` with no initializer is left out**, at either scope: it
        // declares an object with linkage and defines nothing (C17 6.9.2 p1),
        // so neither 6.7 p7, which is about an object with no linkage, nor
        // 6.9.2 p2, which is about a tentative definition, asks its type to
        // be complete. `extern int a[];` is how a header names an array
        // another unit defines. Mutation: keep it; `an_extern_of_an_incomplete_type_is_a_declaration`
        // reports `SC0315`.
        //
        // **A `static` one at file scope is held to its declarator**: 6.9.2
        // p3 forbids a tentative definition with internal linkage an
        // incomplete type, where one with external linkage may be completed
        // later in the unit.
        let declares_only = |storage: &Option<Storage>, d: &InitDeclarator| {
            *storage == Some(Storage::Extern) && d.init.is_none()
        };
        let mut objects: Vec<(&Declaration, bool, bool)> = Vec::new();
        let mut internal: Vec<&Declaration> = Vec::new();
        for item in ast.items() {
            match item {
                Item::Declaration {
                    declarators,
                    storage,
                    ..
                } => {
                    objects.extend(
                        declarators
                            .iter()
                            .filter(|d| !declares_only(storage, d))
                            .map(|d| (&d.declaration, true, d.init.is_some())),
                    );
                    if *storage == Some(Storage::Static) {
                        internal.extend(
                            declarators
                                .iter()
                                .filter(|d| d.init.is_none())
                                .map(|d| &d.declaration),
                        );
                    }
                }
                Item::Function(function) => {
                    let Type::Function {
                        returns,
                        parameters,
                        ..
                    } = ast.ty(function.ty)
                    else {
                        continue;
                    };
                    if let Parameters::Prototype(parameters) = parameters {
                        // `false`, since a parameter has no linkage and its
                        // type is the one written in the declarator.
                        objects.extend(parameters.iter().map(|d| (d, false, false)));
                    }
                    self.report_incomplete_return(ast, *returns, function.name, diagnostics);
                }
                Item::Error { .. } => {}
            }
        }
        for id in ast.stmt_ids() {
            if let Stmt::Declaration {
                declarators,
                storage,
                ..
            } = ast.stmt(id)
            {
                objects.extend(
                    declarators
                        .iter()
                        .filter(|d| !declares_only(storage, d))
                        .map(|d| (&d.declaration, false, d.init.is_some())),
                );
            }
        }
        // Mutation: skip this loop; `a_static_tentative_definition_of_an_incomplete_type_is_refused`
        // exits zero.
        for declaration in internal {
            let ty = declaration.ty;
            let incomplete = match ast.ty(ty) {
                Type::Array { length: None, .. } => true,
                Type::Struct { .. } => !self.resolution.complete(ast, ty),
                _ => false,
            };
            if !incomplete {
                continue;
            }
            let at = declaration.name.unwrap_or(declaration.span);
            diagnostics.report(
                Diagnostic::error(format!(
                    "`{}` has incomplete type `{}`",
                    self.sources.snippet(at),
                    self.spelled(ast, ty)
                ))
                .with_code(INCOMPLETE_OBJECT)
                .with_label(Label::primary(at, "declared here"))
                .with_note(
                    "a tentative definition with internal linkage has a complete type where it is written (C17 6.9.2 p3)",
                ),
            );
        }
        for (declaration, at_file_scope, initialized) in objects {
            let ty = declaration.ty;
            let complete = match ast.ty(ty) {
                // Already refused under `SC0310`, as something that cannot be
                // initialized, and a second report would be about one fault.
                Type::Void if initialized => continue,
                // C17 6.2.5 p19: "an incomplete object type that cannot be
                // completed".
                Type::Void => false,
                // Complete where it is written, which a definition and every
                // struct with no tag is; or, at file scope, defined anywhere.
                Type::Struct { .. } => {
                    self.resolution.complete(ast, ty)
                        || (at_file_scope && {
                            let tag = self.resolution.tag(self.resolution.bound(ast, ty));
                            self.resolution.tag_declaration(tag).defined.is_some()
                        })
                }
                // An array of unknown size (C17 6.2.5 p22). At file scope,
                // with no initializer, it is a tentative definition that
                // 6.9.2 p2 completes, as p5's `int i[];` shows, and one with
                // an initializer has its own rules (6.7.9 p16 and p22), so
                // only a block's uninitialized one is incomplete where 6.7 p7
                // asks. Before this the lowering refused it as this compiler's
                // gap, under `SC0304`.
                // One whose element 6.7.6.2 p1 forbids is already refused, under
                // `SC0309`, and a second report would be about the same fault.
                Type::Array {
                    length: None,
                    element,
                    ..
                } => {
                    if at_file_scope || initialized || self.forbidden_element(ast, *element) {
                        continue;
                    }
                    false
                }
                _ => continue,
            };
            if complete {
                continue;
            }
            let spelled = self.spelled(ast, ty);
            // An unnamed parameter has no name to put in the message, and its
            // span is its specifier, which would say `void` has type `void`.
            let message = match declaration.name {
                Some(name) => format!(
                    "`{}` has incomplete type `{spelled}`",
                    self.sources.snippet(name)
                ),
                None => format!("a parameter has incomplete type `{spelled}`"),
            };
            let at = declaration.name.unwrap_or(declaration.span);
            diagnostics.report(
                Diagnostic::error(message)
                    .with_code(INCOMPLETE_OBJECT)
                    .with_label(Label::primary(at, "declared here"))
                .with_note(if at_file_scope {
                    "an object at file scope has a type completed by the end of the translation unit (C17 6.9.2 p2)"
                } else {
                    "an object with no linkage has a complete type by the end of its declarator (C17 6.7 p7), and so does a parameter of a definition (6.7.6.3 p4)"
                }),
            );
        }
    }

    /// Report `returns` under [`INCOMPLETE_OBJECT`] if it is a struct not
    /// complete where it was written: what a definition returns (C17 6.9.1
    /// p3). `at` is the definition's name.
    fn report_incomplete_return(
        &self,
        ast: &Ast,
        returns: TypeId,
        at: Span,
        diagnostics: &mut DiagnosticSink,
    ) {
        if !matches!(ast.ty(returns), Type::Struct { .. }) || self.resolution.complete(ast, returns)
        {
            return;
        }
        let spelled = self.spelled(ast, returns);
        diagnostics.report(
            Diagnostic::error(format!(
                "`{}` returns incomplete type `{spelled}`",
                self.sources.snippet(at)
            ))
            .with_code(INCOMPLETE_OBJECT)
            .with_label(Label::primary(at, "defined here"))
            .with_note("a function definition returns `void` or a complete type (C17 6.9.1 p3)"),
        );
    }

    /// Report every lvalue whose type is a struct not complete where it is
    /// written, under [`INCOMPLETE_OBJECT`], unless C does not convert it.
    ///
    /// The lvalues that can be one are a name, `*e`, and a subscript with an
    /// operand this stage left untyped, whose pointee `subscript` could not
    /// ask about; a subscript with both operands typed asked already, and a
    /// member access asks for its struct complete. Of those, C17 6.3.2.1 p2
    /// converts every one to its value except the operand of `&`, of `++`
    /// and `--`, the left operand of `.` and of an assignment. The first and
    /// the `.` base read nothing and are not reported. The other two need a
    /// modifiable lvalue (6.5.16 p2, 6.5.2.4 p1, 6.5.3.1 p1), which one of
    /// incomplete type is not (6.3.2.1 p1), and are reported as such. Every
    /// other is a value read, which p2 leaves undefined for an incomplete
    /// type.
    ///
    /// Decided by exclusion rather than by a list of the places a value is
    /// read: a place forgotten in such a list would accept the program in
    /// silence, and one forgotten here reports valid C, visibly. `sizeof`
    /// joins the operands that read nothing when it is read. See the design
    /// comment on #424.
    fn check_incomplete_values(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        let mut unread = HashSet::new();
        let mut modified: HashMap<ExprId, &str> = HashMap::new();
        for id in ast.expr_ids() {
            match ast.expr(id) {
                Expr::Unary {
                    op: UnOp::AddrOf,
                    operand,
                    ..
                } => {
                    unread.insert(*operand);
                }
                Expr::Member {
                    base, arrow: false, ..
                } => {
                    unread.insert(*base);
                }
                Expr::Unary {
                    op: UnOp::PreInc | UnOp::PreDec,
                    operand,
                    ..
                } => {
                    modified.insert(*operand, "C17 6.5.3.1 p1 and 6.3.2.1 p1");
                }
                Expr::Unary {
                    op: UnOp::PostInc | UnOp::PostDec,
                    operand,
                    ..
                } => {
                    modified.insert(*operand, "C17 6.5.2.4 p1 and 6.3.2.1 p1");
                }
                Expr::Assign { place, .. } => {
                    modified.insert(*place, "C17 6.5.16 p2 and 6.3.2.1 p1");
                }
                _ => {}
            }
        }
        for id in ast.expr_ids() {
            let lvalue = match ast.expr(id) {
                Expr::Identifier { .. }
                | Expr::Unary {
                    op: UnOp::Deref, ..
                } => true,
                Expr::Subscript { base, index, .. } => {
                    self.types[base.index()].is_none() || self.types[index.index()].is_none()
                }
                _ => false,
            };
            let Some(ty) = self.types[id.index()] else {
                continue;
            };
            if !lvalue
                || !matches!(ast.ty(ty), Type::Struct { .. })
                || unread.contains(&id)
                || self.resolution.complete_at(ast, ty, id)
            {
                continue;
            }
            let spelled = self.spelled(ast, ty);
            let (message, note) = match modified.get(&id) {
                Some(note) => (
                    format!("`{spelled}` is not complete here, and cannot be modified"),
                    *note,
                ),
                None => (
                    format!("`{spelled}` is not complete here, and its value cannot be read"),
                    "C17 6.3.2.1 p2",
                ),
            };
            diagnostics.report(
                Diagnostic::error(message)
                    .with_code(INCOMPLETE_OBJECT)
                    .with_label(Label::primary(
                        ast.expr(id).span(),
                        format!("this is `{spelled}`"),
                    ))
                    .with_note(note),
            );
        }
    }

    /// Whether `ty` is a variable length array type: an array, any of whose
    /// lengths down the chain of elements has no value. C17 6.7.6.2 p4 makes
    /// an array whose element is one a variable length array too, so
    /// `int a[2][n]` is one although its own length is a constant.
    fn is_variable_length(&self, ast: &Ast, ty: TypeId) -> bool {
        let mut current = ty;
        while let Type::Array {
            element, length, ..
        } = ast.ty(current)
        {
            if length.is_some_and(|length| self.values[length.index()].is_none()) {
                return true;
            }
            current = *element;
        }
        false
    }

    /// Whether C17 6.7.6.2 p1 forbids `element` as an array's element: "The
    /// element type shall not be an incomplete or function type."
    ///
    /// One answer for the two places that ask: `report_array`, which refuses
    /// the array for it, and `check_complete_objects`, which leaves such an
    /// array to that refusal rather than reporting one fault twice.
    fn forbidden_element(&self, ast: &Ast, element: TypeId) -> bool {
        match ast.ty(element) {
            Type::Void | Type::Array { length: None, .. } | Type::Function { .. } => true,
            // Complete where the array is written (C17 6.7.2.1 p8).
            Type::Struct { .. } => !self.resolution.complete(ast, element),
            Type::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            }
            | Type::Pointer(_)
            | Type::VaList
            | Type::Array {
                length: Some(_), ..
            } => false,
        }
    }

    /// How many bytes an object of `ty` takes on the target, where this stage
    /// can say: an integer, a pointer, an array of a constant length, and a
    /// struct, laid out by the IR's one rule (`safec_ir::record::lay_out`, see
    /// ADR-0124). `None` for what is no object type, a struct with no
    /// definition, and an array of no constant length.
    fn bytes_of(&self, ast: &Ast, ty: TypeId) -> Option<u128> {
        match *ast.ty(ty) {
            Type::Integer { rank, sign } => {
                Some(u128::from(self.target.integer(rank, sign).bits() / 8))
            }
            Type::Pointer(_) => Some(u128::from(self.target.pointer_bits() / 8)),
            Type::VaList => Some(u128::from(self.target.va_list().0)),
            Type::Array {
                element, length, ..
            } => {
                let count = u128::try_from(self.values[length?.index()]?).ok()?;
                count.checked_mul(self.bytes_of(ast, element)?)
            }
            Type::Struct { .. } => self
                .struct_layout(ast, ty)
                .map(|(size, _)| u128::from(size)),
            Type::Void | Type::Function { .. } => None,
        }
    }

    /// The size and the alignment in bytes of the struct `ty`, from its
    /// definition's members, by the IR's rule. `None` where it has no
    /// definition or a member has no size here.
    fn struct_layout(&self, ast: &Ast, ty: TypeId) -> Option<(u64, u64)> {
        let definition = self
            .resolution
            .definition(self.resolution.tag(self.resolution.bound(ast, ty)))?;
        let Type::Struct {
            union,
            members: Some(members),
            ..
        } = ast.ty(definition)
        else {
            return None;
        };
        let fields = members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                // A flexible array member takes no bytes, and aligns the
                // struct as its element does (C17 6.7.2.1 p18, #1208).
                // Mutation: size it as one element; `sizeof` in
                // `a_flexible_array_member_holds_what_was_allocated` in
                // `tests/run.rs` is 8. Mutation: align it to 4 whatever its
                // element; the `long long` one there is 4 short.
                let (size, align) = match ast.ty(member.ty) {
                    Type::Array {
                        element,
                        length: None,
                        ..
                    } if index + 1 == members.len() => (0, self.size_and_align(ast, *element)?.1),
                    _ => self.size_and_align(ast, member.ty)?,
                };
                Some(safec_ir::record::Field {
                    size,
                    align,
                    width: match member.width {
                        Some(width) => Some(self.width_of(width)?),
                        None => None,
                    },
                    named: member.name.is_some(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        // The IR's rule, so `sizeof` and the layout the backend writes are
        // one answer (ADR-0124).
        let laid = safec_ir::record::lay_out(self.target, *union, &fields)?;
        Some((laid.size, laid.align))
    }

    /// The size and the alignment in bytes of `ty`, **each struct below it
    /// laid out once**: asked as [`Self::bytes_of`] and [`Self::align_of`]
    /// apart, a member of struct type laid out the one below it twice, so
    /// twenty-two structs each holding the last took eight seconds to size
    /// (#1202 review). Mutation: ask the two apart again;
    /// `a_struct_nested_deep_is_sized_in_time` does not finish.
    fn size_and_align(&self, ast: &Ast, ty: TypeId) -> Option<(u64, u64)> {
        match *ast.ty(ty) {
            Type::Struct { .. } => self.struct_layout(ast, ty),
            Type::Array {
                element, length, ..
            } => {
                let (size, align) = self.size_and_align(ast, element)?;
                let count = u64::try_from(self.values[length?.index()]?).ok()?;
                Some((size.checked_mul(count)?, align))
            }
            Type::Integer { .. }
            | Type::Pointer(_)
            | Type::VaList
            | Type::Void
            | Type::Function { .. } => Some((
                u64::try_from(self.bytes_of(ast, ty)?).ok()?,
                self.align_of(ast, ty)?,
            )),
        }
    }

    /// The alignment in bytes of an object of `ty` on the target, as
    /// `TranslationUnit::align_of` answers it.
    fn align_of(&self, ast: &Ast, ty: TypeId) -> Option<u64> {
        match *ast.ty(ty) {
            Type::Integer { rank, .. } => Some(self.target.alignment(Some(rank))),
            Type::Pointer(_) => Some(self.target.alignment(None)),
            Type::VaList => Some(self.target.va_list().1),
            Type::Array { element, .. } => self.align_of(ast, element),
            Type::Struct { .. } => self.struct_layout(ast, ty).map(|(_, align)| align),
            Type::Void | Type::Function { .. } => None,
        }
    }

    /// Report one array of a declarator at `at` that C17 6.7.6.2 p1 forbids,
    /// or, at file scope or as a member, whose length p2, 6.7.2.1 p9 and 6.6 p4
    /// forbid, and say
    /// whether it was one.
    fn report_array(
        &self,
        ast: &Ast,
        at: Span,
        element: TypeId,
        length: Option<ExprId>,
        place: Where,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if self.forbidden_element(ast, element) {
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
        // C17 6.7.2.1 p3: a struct with a flexible array member is no
        // element of an array, since where the next element begins would
        // depend on how long the last one's array is.
        if self.resolution.has_flexible_array_member(ast, element) {
            let spelled = self.spelled(ast, element);
            diagnostics.report(
                Diagnostic::error(format!(
                    "an array cannot have `{spelled}` as its element, which has a flexible array member"
                ))
                .with_code(DECLARATOR)
                .with_label(Label::primary(at, "declared here"))
                .with_note("C17 6.7.2.1 p3"),
            );
            return true;
        }

        let Some(length) = length else {
            return false;
        };
        // "the expression shall have an integer type", and an untyped one
        // is asked nothing, as `subscript` asks nothing of it. `p - q` was
        // one until #638 typed it; none is measured now (#1298).
        let ty = self.types[length.index()];
        if let Some(ty) = ty.filter(|&ty| {
            !matches!(
                ast.ty(ty),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _
                }
            )
        }) {
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
        // **No object larger than this target can address**: a local one
        // moves the stack pointer by its size, and past the address space
        // that wraps, so the program writes outside its own stack where
        // every access is inside the array. Fewer than 2^61 bytes on a 64-bit
        // target, where the size in bits has to fit 64 bits, and fewer than
        // 2^32 on a 32-bit one, which is also where `clang` stops. C17 5.2.4.1
        // asks only that an object of 65535 bytes be allowed. Mutation: skip
        // this; `an_array_larger_than_the_target_can_address_is_refused`
        // compiles.
        if let Some(count) = self.values[length.index()] {
            let bytes = self
                .bytes_of(ast, element)
                .and_then(|each| u128::try_from(count).ok()?.checked_mul(each));
            let limit = 1u128 << self.target.pointer_bits().min(61);
            if let Some(bytes) = bytes.filter(|&bytes| bytes >= limit) {
                diagnostics.report(
                    Diagnostic::error("an array this large cannot be an object on this target")
                        .with_code(DECLARATOR)
                        .with_label(Label::primary(
                            ast.expr(length).span(),
                            format!("this makes it {bytes} bytes"),
                        ))
                        .with_note(format!(
                            "an object here is smaller than {limit} bytes, the most this target can address"
                        )),
                );
                return true;
            }
        }
        // At file scope a length has to be a constant with a value. Untyped,
        // it was asked nothing above and is asked nothing here: an undeclared
        // name has been reported already, and a length this compiler left
        // untyped goes unasked. `p - q` was the one a valid program could
        // write, until #638 typed it (#1298).
        if !matches!(place, Where::Inner) && ty.is_some() && self.values[length.index()].is_none() {
            let at = ast.expr(length).span();
            diagnostics.report(if self.undefined[length.index()] {
                Diagnostic::error("the length of an array has no defined value")
                    .with_code(DECLARATOR)
                    .with_label(Label::primary(at, "this overflows its type, divides by zero, or shifts too far"))
                    .with_note(
                        "a constant expression evaluates to a value its type can hold (C17 6.6 p4)",
                    )
            } else if matches!(place, Where::Member) {
                Diagnostic::error(
                    "a member of a struct cannot have an array length that is not a constant",
                )
                .with_code(DECLARATOR)
                .with_label(Label::primary(at, "this is not a constant"))
                .with_note("a member of a structure has no variably modified type (C17 6.7.6.2 p2 and 6.7.2.1 p9)")
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
            // 6.6 p6 allows a cast of an arithmetic operand to an integer
            // type, and `sizeof` of anything this check sizes. A cast to a
            // pointer is no integer constant at all, so it has no value
            // rather than an undefined one.
            Expr::Cast { ty, operand, .. } => {
                matches!(ast.ty(ty), Type::Integer { .. }) && known(operand)
            }
            // A `sizeof` asked here has no value, which is one of a variable
            // length array, sized when it runs: no constant at all (6.6 p6
            // excepts it), rather than one with an undefined value (#1209).
            // Mutation: answer `true`;
            // `a_sizeof_of_a_variable_length_array_is_no_constant` says its
            // `static` initializer overflows.
            Expr::SizeofType { .. } | Expr::SizeofExpr { .. } => false,
            Expr::Conditional {
                condition,
                then,
                otherwise,
                ..
            } => known(condition) && known(then) && known(otherwise),
            Expr::Number { .. }
            | Expr::Character { .. }
            | Expr::String { .. }
            | Expr::Length { .. }
            | Expr::Identifier { .. }
            // Its value is known or it has none, as a constant's is.
            | Expr::Enumerator { .. }
            | Expr::Assign { .. }
            | Expr::Comma { .. }
            | Expr::Subscript { .. }
            | Expr::Member { .. }
            | Expr::Call { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => false,
        }
    }

    /// The type of the member access `id`, C17 6.5.2.3: the type of the
    /// member it names, which is recorded in `members`.
    ///
    /// Three things are asked, in the order a reader would: whether the base
    /// is the shape the operator takes (p1, `OPERANDS`), whether its struct
    /// is complete where the access is written, since an incomplete struct
    /// has no members yet (6.7.2.1 p8, `INCOMPLETE_OBJECT`), and whether the
    /// struct has a member of that name (p1, `NO_MEMBER`). The member is
    /// found by spelling among the members of the definition the base's tag
    /// names, so `s.a` with `s` declared before `struct S`'s definition is
    /// typed below it.
    fn member(
        &mut self,
        ast: &Ast,
        id: ExprId,
        (base, member, arrow, span): (ExprId, Span, bool, Span),
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let operator = if arrow { "->" } else { "." };
        let Some(base_ty) = self.types[base.index()] else {
            // A base with a fault reported within it, `nowhere.x` or `a.b.c`
            // with no `b`, says nothing more, because the run already fails
            // and the fault is usually why the base has no type. Not always:
            // a base untyped by this compiler's own gap beside the fault is
            // not said until the fault is fixed, which is the trade `binary`
            // makes beside a marked operand. Any other base with no type was
            // left untyped by this compiler, and without its type whether the
            // access is C cannot be told. `(p - q).a` was the example until
            // #638 typed a difference; none is measured now (#1298). So it is
            // declined rather
            // than passed over, and the note claims neither. An access that
            // says nothing here is marked through its base, by
            // `subexpressions`.
            if !self.reported_within[base.index()] {
                diagnostics.report(
                    Diagnostic::error(format!("cannot check `{operator}` yet"))
                        .with_code(NOT_YET)
                        .with_label(Label::primary(span, "a member of something with no type"))
                        .with_note(
                            "the base has no type here, which is a gap in this compiler: whether this access is valid C is not checked",
                        ),
                );
            }
            return None;
        };

        // p1: `.` takes a struct and `->` a pointer to one. An array of
        // structs is one after 6.3.2.1 p3, so `a->m` is C.
        let structure = match ast.ty(base_ty) {
            Type::Struct { .. } => (!arrow).then_some(base_ty),
            Type::Pointer(element) | Type::Array { element, .. } => {
                (arrow && matches!(ast.ty(*element), Type::Struct { .. })).then_some(*element)
            }
            Type::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            }
            | Type::Void
            | Type::VaList
            | Type::Function { .. } => None,
        };
        let Some(structure) = structure else {
            let spelled = self.spelled(ast, base_ty);
            let wanted = if arrow {
                "a pointer to a struct"
            } else {
                "a struct"
            };
            diagnostics.report(
                Diagnostic::error(format!(
                    "`{operator}` needs {wanted}, and this is `{spelled}`"
                ))
                .with_code(OPERANDS)
                .with_label(Label::primary(
                    ast.expr(base).span(),
                    format!("this is `{spelled}`"),
                ))
                .with_note("C17 6.5.2.3 p1"),
            );
            return None;
        };

        let tag = self.resolution.tag(self.resolution.bound(ast, structure));
        let spelled = self.spelled(ast, structure);
        // Complete here means its definition closed above, so it has one.
        let definition = self
            .complete_here(ast, structure)
            .then(|| self.resolution.definition(tag))
            .flatten();
        let Some(definition) = definition else {
            diagnostics.report(
                Diagnostic::error(format!(
                    "`{spelled}` is not complete here, and has no members yet"
                ))
                .with_code(INCOMPLETE_OBJECT)
                .with_label(Label::primary(
                    ast.expr(base).span(),
                    format!("this is `{}`", self.spelled(ast, base_ty)),
                ))
                .with_note("C17 6.5.2.3 p1 and 6.7.2.1 p8"),
            );
            return None;
        };
        let Type::Struct {
            members: Some(members),
            ..
        } = ast.ty(definition)
        else {
            unreachable!("a definition the resolver recorded has members");
        };

        let name = self.sources.snippet(member);
        let Some((path, ty)) = self.find_member(ast, members, name) else {
            let mut diagnostic =
                Diagnostic::error(format!("no member named `{name}` in `{spelled}`"))
                    .with_code(NO_MEMBER)
                    .with_label(Label::primary(
                        member,
                        format!("not a member of `{spelled}`"),
                    ))
                    .with_note("C17 6.5.2.3 p1");
            if let Some(defined) = self.resolution.tag_declaration(tag).defined {
                diagnostic = diagnostic.with_label(Label::secondary(
                    defined,
                    format!("`{spelled}` is defined here"),
                ));
            }
            diagnostics.report(diagnostic);
            return None;
        };
        self.members[id.index()] = Some(Member { definition, path });
        Some(ty)
    }

    /// The type an access to a member of type `ty` is typed at: `ty`, but
    /// for a bit-field, the type it promotes to (#1206). C17 6.3.1.1 p2
    /// promotes one of `int`, `signed int` or `unsigned int` whose values
    /// all fit an `int` to `int`, and otherwise to `unsigned int`, and
    /// leaves one of any other type to the implementation: `clang` promotes
    /// any integer bit-field narrower than `int` to `int`, one exactly as
    /// wide to `unsigned int` where it is unsigned, and a wider one not at
    /// all, and this does as `clang` does, since a program built here is
    /// linked with ones built there. Nothing can tell the two types apart
    /// but promotion, since a bit-field has no address and no `sizeof`, so
    /// the access is typed as what it promotes to and every rule that
    /// promotes it gets it right without asking. A `const` one stays `const`.
    ///
    /// Mutation: answer `ty`; `a_bit_field_is_read_and_written_within_its_unit`
    /// in `tests/run.rs` fails at `promoted`. Mutation: promote only
    /// `unsigned int`; it fails at `promoted_wide`.
    fn read_as(&mut self, ast: &mut Ast, id: ExprId, ty: TypeId) -> TypeId {
        let Some(width) = self
            .bit_field(ast, id)
            .and_then(|width| self.width_of(width))
        else {
            return ty;
        };
        let Some((rank, sign)) = integer_of(ast, ty) else {
            return ty;
        };
        let int_bits = u64::from(self.target.integer(Rank::Int, Sign::Signed).bits());
        let unsigned = !self.target.integer(rank, sign).signed();
        let promoted = match width.cmp(&int_bits) {
            std::cmp::Ordering::Less => Type::INT,
            std::cmp::Ordering::Equal if unsigned => Type::Integer {
                rank: Rank::Int,
                sign: Sign::Unsigned,
            },
            std::cmp::Ordering::Equal => Type::INT,
            std::cmp::Ordering::Greater => return ty,
        };
        let promoted = ast.push_type(promoted);
        if ast.is_const(ty) {
            ast.push_const(promoted)
        } else {
            promoted
        }
    }

    /// The width expression of the bit-field the member access `id` names,
    /// or `None` where it names no bit-field or is no member access.
    fn bit_field(&self, ast: &Ast, id: ExprId) -> Option<ExprId> {
        let member = self.members[id.index()].as_ref()?;
        let mut definition = member.definition;
        let (last, through) = member.path.split_last()?;
        for &index in through {
            let Type::Struct {
                members: Some(members),
                ..
            } = ast.ty(definition)
            else {
                return None;
            };
            definition = members[index].ty;
        }
        let Type::Struct {
            members: Some(members),
            ..
        } = ast.ty(definition)
        else {
            return None;
        };
        members[*last].width
    }

    /// A bit-field's width, where its expression is a constant this check
    /// worked out and it is not negative: `None` otherwise, which
    /// `check_bit_fields` reports.
    fn width_of(&self, width: ExprId) -> Option<u64> {
        u64::try_from(self.values[width.index()]?).ok()
    }

    /// Refuse `what` applied to a bit-field, which `clause` forbids, and say
    /// whether it was one (#1206).
    fn refuse_bit_field(
        &self,
        ast: &Ast,
        operand: ExprId,
        what: &str,
        clause: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if self.bit_field(ast, operand).is_none() {
            return false;
        }
        diagnostics.report(
            Diagnostic::error(format!("{what} cannot be applied to a bit-field"))
                .with_code(OPERANDS)
                .with_label(Label::primary(
                    ast.expr(operand).span(),
                    "this is a bit-field",
                ))
                .with_note(clause.to_owned()),
        );
        true
    }

    /// Hold each bit-field's width to C17 6.7.2.1 p4 and p5 (#1206): an
    /// integer constant expression, not negative, no wider than its type, and
    /// zero only where the bit-field has no name; and its type an integer
    /// type, which p5 leaves the integer types beyond `int` to the
    /// implementation to allow, and `clang` allows.
    fn check_bit_fields(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        for ty in ast.type_ids() {
            let Type::Struct {
                members: Some(members),
                ..
            } = ast.ty(ty)
            else {
                continue;
            };
            for member in members {
                let Some(width) = member.width else {
                    continue;
                };
                let at = ast.expr(width).span();
                let unfit = |why: String| {
                    Diagnostic::error(format!("this bit-field's width {why}"))
                        .with_code(UNFIT_MEMBER)
                        .with_label(Label::primary(at, "the width"))
                        .with_note("C17 6.7.2.1 p4")
                };
                let Some(bits) = integer_of(ast, member.ty)
                    .map(|(rank, sign)| u64::from(self.target.integer(rank, sign).bits()))
                else {
                    let spelled = self.spelled(ast, member.ty);
                    diagnostics.report(
                        Diagnostic::error(format!("a bit-field cannot have type `{spelled}`"))
                            .with_code(UNFIT_MEMBER)
                            .with_label(Label::primary(
                                member.name.unwrap_or(member.span),
                                "a bit-field has an integer type",
                            ))
                            .with_note("C17 6.7.2.1 p5"),
                    );
                    continue;
                };
                match self.values[width.index()] {
                    None if !self.reported_within[width.index()] => {
                        diagnostics.report(unfit("is not an integer constant".to_owned()));
                    }
                    None => {}
                    Some(value) if value < 0 => {
                        diagnostics.report(unfit(format!("is negative, {value}")));
                    }
                    Some(value) if value > i128::from(bits) => diagnostics.report(unfit(format!(
                        "is {value}, wider than its type's {bits} bits"
                    ))),
                    Some(0) if member.name.is_some() => {
                        diagnostics.report(unfit(
                            "is zero, and only a bit-field with no name may be".to_owned(),
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
    }

    /// Where the member spelled `name` is among `members`, as a path of
    /// indices, and its type: a named member's own index, or an anonymous
    /// member's followed by the path within it (C17 6.7.2.1 p13, #1207).
    /// The first match in the order written; a name declared twice across
    /// them was reported by the resolver, which does not stop this check.
    ///
    /// A recursion, bounded by how deep the parser nests a member list.
    /// Mutation: skip anonymous members; `an_anonymous_member_is_reached_through_its_container`'s
    /// `read` is `SC0316`.
    fn find_member(
        &self,
        ast: &Ast,
        members: &[Declaration],
        name: &str,
    ) -> Option<(Vec<usize>, TypeId)> {
        for (index, declared) in members.iter().enumerate() {
            match declared.name {
                Some(declared_name) if self.sources.snippet(declared_name) == name => {
                    return Some((vec![index], declared.ty));
                }
                Some(_) => {}
                None => {
                    let Type::Struct {
                        members: Some(inner),
                        ..
                    } = ast.ty(declared.ty)
                    else {
                        continue;
                    };
                    if let Some((mut path, ty)) = self.find_member(ast, inner, name) {
                        path.insert(0, index);
                        return Some((path, ty));
                    }
                }
            }
        }
        None
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
            Stmt::Do {
                body, condition, ..
            } => {
                self.receivers_in(ast, *body, returning, diagnostics);
                self.receivers
                    .insert(*condition, Receiving::Condition { keyword: "do" });
            }
            Stmt::For {
                start,
                condition,
                step: _,
                body,
                span: _,
            } => {
                // A declaration's initializers are received as a block's are,
                // so `for (int *p = 1; ...)` is held to C17 6.7.9 p11.
                match *start {
                    Some(ForStart::Declaration(declaration)) => {
                        self.receivers_in(ast, declaration, returning, diagnostics);
                    }
                    Some(ForStart::Expression(_)) | None => {}
                }
                if let Some(condition) = *condition {
                    self.receivers
                        .insert(condition, Receiving::Condition { keyword: "for" });
                }
                self.receivers_in(ast, *body, returning, diagnostics);
            }
            Stmt::Labeled { body, .. } | Stmt::Case { body, .. } | Stmt::Default { body, .. } => {
                self.receivers_in(ast, *body, returning, diagnostics)
            }
            Stmt::Switch {
                condition, body, ..
            } => {
                self.receivers.insert(*condition, Receiving::Switch);
                self.receivers_in(ast, *body, returning, diagnostics);
            }
            Stmt::Expression { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Goto { .. }
            | Stmt::Error { .. } => {}
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
            // A list's items are held to their subobjects by
            // `Checker::check_list`, once every expression is typed.
            let Some(init) = declarator.init.as_ref().and_then(Initializer::expr) else {
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
        self.at = Some(id);
        // Cloned rather than borrowed, because working out the type can push
        // into the arena that the borrow would be from. A `Call`'s argument
        // list is the only part of this that allocates.
        let expr = ast.expr(id).clone();

        match expr {
            Expr::Number { span } => self.constant(ast, id, span, diagnostics),
            Expr::Character { span } => {
                // C17 6.4.4.4 p10: type `int`, and the value of a `char`
                // whose object representation is the character's, so
                // `'\xff'` is -1 where `char` is signed and 255 where it is
                // not. The parser read the one character; this reads it again.
                let text = self.sources.snippet(span);
                let body = &text[1..text.len() - 1];
                let value = crate::literal::unescape(body)
                    .ok()
                    .and_then(|values| values.first().copied())
                    .expect("the parser keeps only a constant of one character it reads");
                let value = i128::from(value);
                let character = self.target.integer(Rank::Char, Sign::Plain);
                let value = if character.signed() && value > character.max() {
                    value - (1i128 << character.bits())
                } else {
                    value
                };
                self.values[id.index()] = Some(value);
                Some(self.int)
            }
            // C17 6.4.5 p6: an array of `char`, as long as the bytes the parser
            // counted, an `Expr::Length`, so that `&"ab"` is a `char (*)[3]`
            // to every rule that compares or spells one. Mutation: give it no
            // length; `a_string_literal_s_address_is_a_pointer_to_its_whole_array`
            // accepts `char (*)[9]`.
            Expr::String { length, .. } => {
                let element = ast.push_type(Type::CHAR);
                Some(ast.push_type(Type::Array {
                    element,
                    length: Some(length),
                    written: None,
                }))
            }
            // The parser counted it. Typed `int`, as a written length that
            // fits one is, since nothing but an array's type reads it.
            Expr::Length { value, .. } => {
                self.values[id.index()] = Some(i128::from(value));
                Some(self.int)
            }
            // C17 6.7.2.2 p3: an enumeration constant has type `int`. Its
            // value is `evaluate`'s, from the enumerator's.
            Expr::Enumerator { .. } => Some(self.int),
            Expr::Identifier { .. } => {
                // A function is typed by its standing declaration, not the
                // one the use resolved to: see `Resolution::standing`. An
                // object is typed by the declaration in scope, as C types
                // it, since `int (*fp)();` holds whatever compatible
                // function was assigned to it, and a later `int (*fp)(int);`
                // does not change what a call written before it may pass.
                let resolved = self.resolution.resolved(id)?;
                let ty = match ast.ty(self.resolution.binding(resolved).ty) {
                    Type::Function { .. } => {
                        let declared = self.resolution.standing(resolved);
                        self.resolution.binding(declared).ty
                    }
                    _ => self.resolution.binding(resolved).ty,
                };
                Some(ty)
            }
            Expr::Member {
                base,
                member,
                arrow,
                span,
            } => {
                let ty = self.member(ast, id, (base, member, arrow, span), diagnostics)?;
                let ty = self.read_as(ast, id, ty);
                // C17 6.5.2.3 p3 and p4: a member of a `const` structure is
                // `const`, through `.` and through `->` alike, so `s.a = 1`
                // with `const struct S s` writes what may not be written.
                // Found by review.
                let base_ty = self.types[base.index()]?;
                let qualified = match ast.ty(base_ty) {
                    Type::Pointer(pointee) if arrow => ast.is_const(*pointee),
                    _ => ast.is_const(base_ty),
                };
                Some(if qualified { qualify(ast, ty) } else { ty })
            }
            Expr::Unary { op, operand, .. } => self.unary(ast, id, op, operand, diagnostics),
            Expr::Binary { op, lhs, rhs, .. } => self.binary(ast, id, op, lhs, rhs, diagnostics),
            Expr::Assign {
                op, place, value, ..
            } => {
                // Two rules, because C17 has two: a plain `=` is 6.5.16.1,
                // and a compound assignment is 6.5.16.2, whose constraints
                // are its own. `p += 1` is legal for a pointer and the first
                // rule would report it. Both want a modifiable lvalue on the
                // left, 6.5.16 p2, asked first and for either; a left operand
                // that cannot be written to is one fault, and the type rules
                // are not asked about it, as `++` and `--` do not ask theirs.
                let spelled = match op {
                    None => "=".to_owned(),
                    Some(op) => format!("{}=", op.as_str()),
                };
                let refused = self.refuse_unwritable(
                    ast,
                    place,
                    &format!("the left operand of `{spelled}`"),
                    "C17 6.5.16 p2",
                    diagnostics,
                );
                match op {
                    _ if refused => {}
                    None => self.assignment(ast, place, value, diagnostics),
                    Some(op) => {
                        self.compound_assignment(ast, op, place, value, diagnostics);
                        // 6.5.16.2 p3: `a op= b` is `a = a op b`, computed at
                        // the type `a op b` has, and converted to `a`'s.
                        self.operated[id.index()] = self.operated_at(ast, op, place, value);
                    }
                }
                // 6.5.16 p3: the type is "the type the left operand would have
                // after lvalue conversion", which is the place's own while
                // there are no qualifiers to drop. None for a refused place,
                // as for a refused `++`: an array's type there had `int b[a[0]
                // = 0]` reported again as a length of array type.
                if refused {
                    return None;
                }
                self.types[place.index()]
            }
            // 6.5.17 p2: the result of a comma is the value of the right
            // operand, and its type, which is a value's: an array is a
            // pointer there (6.3.2.1 p3), since the comma and not `sizeof`
            // is its operator, so `sizeof(0, a)` is a pointer's size.
            // Mutation: give the right operand's type undecayed;
            // `sizeof_of_a_comma_is_a_pointer_s_size` answers the array's.
            Expr::Comma { rhs, .. } => {
                let ty = self.types[rhs.index()]?;
                Some(self.decayed(ast, ty))
            }
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
            Expr::SizeofType { ty, span } => self.sizeof(ast, id, ty, span, diagnostics),
            // The operand is typed, as every expression is, and never
            // evaluated (6.5.3.4 p2): nothing lowers it.
            Expr::SizeofExpr { operand, span } => {
                let ty = self.types[operand.index()]?;
                if self.refuse_bit_field(ast, operand, "`sizeof`", "C17 6.5.3.4 p1", diagnostics) {
                    return None;
                }
                self.sizeof(ast, id, ty, span, diagnostics)
            }
            Expr::Cast { ty, operand, span } => self.cast(ast, ty, operand, span, diagnostics),
            Expr::VaOp { op, operands, span } => self.va_op(ast, op, &operands, span, diagnostics),
            Expr::VaArg { list, ty, span } => self.va_arg(ast, list, ty, span, diagnostics),
            Expr::Error { .. } => None,
        }
    }

    /// The type of `sizeof` of `ty`, `size_t` (C17 6.5.3.4 p5), with its
    /// value recorded: the bytes [`Self::bytes_of`] counts, which is what
    /// `clang` answers on the target (`sizeof_is_what_clang_says_it_is`).
    ///
    /// p1 refuses a function and an incomplete type (`SC0306`), a struct
    /// nothing defines among them. A struct is laid out by the IR's rule
    /// ([`Self::bytes_of`]). An array of a length that is not a constant is
    /// sized at run time, which nothing here does yet, and is `SC0304`.
    fn sizeof(
        &mut self,
        ast: &mut Ast,
        id: ExprId,
        ty: TypeId,
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let incomplete = match ast.ty(ty) {
            Type::Function { .. } => Some("a function"),
            Type::Void => Some("`void`, which is incomplete"),
            Type::Array { length: None, .. } => Some("an array of no length, which is incomplete"),
            // **Complete where the `sizeof` is**, not by the end of the
            // file: `struct S; ... sizeof(struct S) ... struct S { int x; };`
            // is p1's incomplete type at the `sizeof` (#1200 review).
            // Mutation: ask whether anything defines it at all;
            // `sizeof_refuses_what_has_no_size`'s `sizeof(struct L)` is
            // accepted.
            Type::Struct { union, .. }
                if !self.complete_here(ast, self.resolution.bound(ast, ty)) =>
            {
                Some(if *union {
                    "a union not complete here, which is incomplete"
                } else {
                    "a struct not complete here, which is incomplete"
                })
            }
            Type::Integer { .. }
            | Type::Pointer(_)
            | Type::VaList
            | Type::Array {
                length: Some(_), ..
            }
            | Type::Struct { .. } => None,
        };
        if let Some(what) = incomplete {
            diagnostics.report(
                Diagnostic::error(format!("`sizeof` cannot be applied to {what}"))
                    .with_code(OPERANDS)
                    .with_label(Label::primary(
                        span,
                        format!("this is `{}`", self.spelled(ast, ty)),
                    ))
                    .with_note("C17 6.5.3.4 p1"),
            );
            return None;
        }
        // **A variable length array is sized when it runs** (C17 6.5.3.4
        // p2, #1209): an array whose length is no constant and whose element
        // has a size, so `sizeof` has `size_t`'s type and no value here, and
        // the lowering multiplies the length it holds by the element's size.
        // Mutation: refuse it as before; the run of
        // `a_variable_length_array_is_sized_when_declared` fails, its
        // `sizeof` refused with `SC0304`. Mutation: drop the element's
        // size from the test; `a_sizeof_of_a_variable_length_array_is_no_constant`
        // loses its refusal of `sizeof` over `int[n][n]`.
        if let Type::Array {
            element,
            length: Some(length),
            ..
        } = *ast.ty(ty)
        {
            if self.values[length.index()].is_none() && self.bytes_of(ast, element).is_some() {
                return Some(self.integer_type(ast, self.target.size_type()));
            }
        }
        let Some(bytes) = self.bytes_of(ast, ty) else {
            diagnostics.report(
                Diagnostic::error(format!(
                    "this compiler cannot size `{}` yet",
                    self.spelled(ast, ty)
                ))
                .with_code(NOT_YET)
                .with_label(Label::primary(span, "this `sizeof`"))
                .with_note(
                    "a gap in this compiler: an array of a length that is not a constant is sized at run time, and a struct is sized here only where every member is",
                ),
            );
            return None;
        };
        // No object is as large, as `report_array` holds a declared one to,
        // so its size has no value in `size_t`. Mutation: skip this;
        // `sizeof_a_type_larger_than_the_target_can_address_is_refused`
        // is accepted.
        let limit = 1u128 << self.target.pointer_bits().min(61);
        if bytes >= limit {
            diagnostics.report(
                Diagnostic::error("`sizeof` a type larger than this target can address")
                    .with_code(DECLARATOR)
                    .with_label(Label::primary(span, format!("this is {bytes} bytes")))
                    .with_note(format!(
                        "an object here is smaller than {limit} bytes, the most this target can address"
                    )),
            );
            return None;
        }
        let size = self.integer_type(ast, self.target.size_type());
        self.values[id.index()] = Some(
            i128::try_from(bytes)
                .expect("an object's size is below the limit `report_array` holds"),
        );
        Some(size)
    }

    /// The type of the cast of `operand` to `ty`, which is `ty`: C17 6.5.4
    /// p2 asks that it be `void`, or a scalar type with a scalar operand.
    /// An array or a function operand is a pointer by then (6.3.2.1 p3 and
    /// p4). Not an lvalue (footnote 104); [`Checker::designates`] says so.
    fn cast(
        &mut self,
        ast: &mut Ast,
        ty: TypeId,
        operand: ExprId,
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let to_scalar = match ast.ty(ty) {
            Type::Void => return Some(ty),
            Type::Integer { .. } | Type::Pointer(_) => true,
            Type::Array { .. } | Type::Function { .. } | Type::Struct { .. } | Type::VaList => {
                false
            }
        };
        let from = self.types[operand.index()]?;
        let from_scalar = match ast.ty(from) {
            Type::Integer { .. }
            | Type::Pointer(_)
            | Type::Array { .. }
            | Type::Function { .. } => true,
            Type::Void | Type::Struct { .. } | Type::VaList => false,
        };
        if to_scalar && from_scalar {
            return Some(ty);
        }
        let (to, from) = (self.spelled(ast, ty), self.spelled(ast, from));
        diagnostics.report(
            Diagnostic::error(format!("cannot cast `{from}` to `{to}`"))
                .with_code(OPERANDS)
                .with_label(Label::primary(span, format!("a `{from}` cast to `{to}`")))
                .with_note("a cast is to `void`, or to a scalar type from one (C17 6.5.4 p2)"),
        );
        None
    }

    fn unary(
        &mut self,
        ast: &mut Ast,
        id: ExprId,
        op: UnOp,
        operand: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let operand_ty = self.types[operand.index()];

        match op {
            // 6.5.3.2 p3: `&x` is a pointer to the type of `x`. The type
            // itself is one nobody wrote, so it is pushed. p1's constraint on
            // the operand is asked first, and a refused operand gives no
            // type, as a refused `++` does: a pointer there had `p =
            // &g().arr` reported again as a pointer of the wrong type.
            UnOp::AddrOf => {
                if self.refuse_unaddressable(ast, operand, diagnostics) {
                    return None;
                }
                // p1 again: a bit-field has no address (#1206).
                if self.refuse_bit_field(ast, operand, "`&`", "C17 6.5.3.2 p1", diagnostics) {
                    return None;
                }
                Some(ast.push_type(Type::Pointer(operand_ty?)))
            }
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
                // 6.3.2.1 p3 makes `a` a pointer to its first element, so
                // `*a` is that element, as `a[0]` is. Left untyped, it was a
                // part of a prototype's length nothing could check, and `*g2
                // = 0` with `g2` an array of arrays went unreported.
                Type::Array { element, .. } => Some(*element),
                // p2: the operand of `*` shall have pointer type. Reported,
                // and `None`: there is no type `*x` would have had. Nothing
                // after the type check reads the tree once it has reported,
                // so no later stage speaks about the missing type.
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Void
                | Type::VaList
                | Type::Struct { .. } => {
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
            //
            // Each also wants a modifiable lvalue, p1 of the same clause,
            // asked first: an operand that cannot be written to gives the
            // expression no type, and its type rules are not asked.
            UnOp::PostInc | UnOp::PostDec => {
                let what = format!("the operand of `{}`", op.as_str());
                if self.refuse_unwritable(ast, operand, &what, "C17 6.5.2.4 p1", diagnostics) {
                    return None;
                }
                let ty = self.increment(ast, op, operand, "C17 6.5.2.4 p2", diagnostics);
                self.operated[id.index()] = self.stepped_at(ast, operand);
                ty
            }
            UnOp::PreInc | UnOp::PreDec => {
                let what = format!("the operand of `{}`", op.as_str());
                if self.refuse_unwritable(ast, operand, &what, "C17 6.5.3.1 p1", diagnostics) {
                    return None;
                }
                let ty = self.increment(ast, op, operand, "C17 6.5.3.1 p2", diagnostics);
                self.operated[id.index()] = self.stepped_at(ast, operand);
                ty
            }
            // 6.5.3.3: the integer promotions of 6.3.1.1 give `+`, `-` and
            // `~` their result's type, and p5 makes `!` an `int` outright.
            // Still `None` for an operand
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
                    Type::Integer {
                        rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                        sign: _,
                    } => true,
                    // `decayed` answers neither an array nor a function, and
                    // each would be the pointer it decays to if it did.
                    Type::Pointer(_) | Type::Array { .. } | Type::Function { .. } => {
                        matches!(op, UnOp::Not)
                    }
                    Type::Void | Type::Struct { .. } | Type::VaList => false,
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
                // p5: `!` is an `int`; `+`, `-` and `~` are the promoted
                // operand's type (p2 to p4).
                match (op, integer_of(ast, ty)) {
                    (UnOp::Plus | UnOp::Minus | UnOp::BitNot, Some(integer)) => {
                        let promoted = self.promoted_of(integer);
                        Some(self.integer_type(ast, promoted))
                    }
                    _ => Some(self.int),
                }
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

        // p1: a real or a pointer operand, which a struct is not.
        if let Type::Struct { .. } = ast.ty(ty) {
            let spelled = self.spelled(ast, ty);
            diagnostics.report(
                Diagnostic::error(format!("`{}` cannot take `{spelled}`", op.as_str()))
                    .with_code(OPERANDS)
                    .with_label(Label::primary(
                        ast.expr(operand).span(),
                        format!("this is `{spelled}`"),
                    )),
            );
            return None;
        }

        if let Type::Pointer(pointee) = ast.ty(ty) {
            if let Some(why) = unsteppable(ast, *pointee, &|ty| self.complete_here(ast, ty)) {
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
    /// type. All of that is asked only where both operands are typed: an
    /// operand this compiler left untyped goes unreported here and untyped
    /// into the lowering. `x[p - q]` was that until #638 gave a difference
    /// `ptrdiff_t`, and is now an integer subscripted by an integer,
    /// refused; no untyped operand is measured now (#1298). The
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
                    pointee_steppable(ast, pointee, true, &|ty| self.complete_here(ast, ty)).err()
                }
                (Some(OperandClass::Arithmetic), Some(OperandClass::Pointer(pointee))) => {
                    pointee_steppable(ast, pointee, false, &|ty| self.complete_here(ast, ty)).err()
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
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Void
                | Type::VaList
                | Type::Array { .. }
                | Type::Function { .. }
                | Type::Struct { .. } => None,
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
    /// Beside an operand this stage could not type, the answer turns on why
    /// it is untyped. An operand can be untyped with nothing reported, as
    /// `p - q` was until #638 gave it `ptrdiff_t`; none is measured now
    /// (#1298). Every operator but `+` and `-` is still `int` there, because
    /// no type would skip the check around it, which kept `int *r = (p - q)
    /// * 2;` its `SC0302` while it was. An operand that is untyped because a fault in it
    /// was reported (see `reported_within`) gives no type, as `unary` does, because
    /// an `int` there had `p = nowhere * 1` reported as an undeclared name
    /// and then as an `int` given to a pointer.
    fn binary(
        &mut self,
        ast: &mut Ast,
        id: ExprId,
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
            let reported = [lhs, rhs].into_iter().any(|operand| {
                self.types[operand.index()].is_none() && self.reported_within[operand.index()]
            });
            return (!reported && !matches!(op, BinOp::Add | BinOp::Sub)).then_some(self.int);
        };
        let nulls = (
            self.is_null_pointer_constant(ast, lhs),
            self.is_null_pointer_constant(ast, rhs),
        );
        if let Some(Err(refused)) = self.binary_operable(ast, op, left, right, nulls) {
            self.report_operands(ast, op, refused, (lhs, left), (rhs, right), diagnostics);
            return None;
        }

        let integers = (integer_of(ast, left), integer_of(ast, right));
        match (op, integers) {
            // 6.5.5 p3, 6.5.6 p4 and 6.5.10 to 6.5.12 p3: the usual
            // arithmetic conversions are performed on the operands, and the
            // result has the type they give.
            (
                BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::Add
                | BinOp::Sub
                | BinOp::BitAnd
                | BinOp::BitXor
                | BinOp::BitOr,
                (Some(left), Some(right)),
            ) => {
                let common = self.usual_of(left, right);
                Some(self.integer_type(ast, common))
            }
            // 6.5.6 p9: a pointer minus a pointer is `ptrdiff_t`, the
            // target's, and `binary_operable` has held the two to compatible
            // object types already (#638). Mutation: answer `None` as
            // before; `the_difference_of_two_pointers_into_one_array_is_its_subscripts`
            // is refused.
            (BinOp::Sub, _)
                if matches!(ast.ty(left), Type::Pointer(_))
                    && matches!(ast.ty(right), Type::Pointer(_)) =>
            {
                Some(self.integer_type(ast, self.target.ptrdiff_type()))
            }
            (BinOp::Add | BinOp::Sub, _) => self.additive(ast, op, left, right),
            // 6.5.7 p3: each operand is promoted, and the result has the
            // promoted left operand's type; the right does not convert it.
            (BinOp::Shl | BinOp::Shr, (Some(left), Some(_))) => {
                let promoted = self.promoted_of(left);
                Some(self.integer_type(ast, promoted))
            }
            // 6.5.8 p3 and 6.5.9 p4: the usual arithmetic conversions are
            // performed on two arithmetic operands, and the result is an
            // `int` (p6, p3); the common type is what the comparison is made
            // at, which is [`Types::operated`].
            (
                BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne,
                (Some(left), Some(right)),
            ) => {
                let common = self.usual_of(left, right);
                self.operated[id.index()] = Some(self.integer_type(ast, common));
                Some(self.int)
            }
            // A pointer compared, and `&&` and `||`, which 6.5.13 p3 and
            // 6.5.14 p3 make `int`. A shift or an arithmetic operator on an
            // operand that is no integer was refused above.
            (
                BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge
                | BinOp::Eq
                | BinOp::Ne
                | BinOp::LogAnd
                | BinOp::LogOr
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::Shl
                | BinOp::Shr
                | BinOp::BitAnd
                | BinOp::BitXor
                | BinOp::BitOr,
                _,
            ) => Some(self.int),
        }
    }

    /// The type a compound assignment `place op= value` computes at
    /// (6.5.16.2 p3): what `place op value` would have, the promoted left
    /// operand's for a shift (6.5.7 p3) and the usual arithmetic
    /// conversions' otherwise. `None` where either is not an integer, which
    /// is a pointer moved by an integer and performed at the pointer.
    fn operated_at(
        &mut self,
        ast: &mut Ast,
        op: BinOp,
        place: ExprId,
        value: ExprId,
    ) -> Option<TypeId> {
        let left = integer_of(ast, self.types[place.index()]?)?;
        let right = integer_of(ast, self.types[value.index()]?)?;
        let at = match op {
            BinOp::Shl | BinOp::Shr => self.promoted_of(left),
            _ => self.usual_of(left, right),
        };
        Some(self.integer_type(ast, at))
    }

    /// The type an increment or a decrement of `operand` computes at: `E +=
    /// 1`'s (6.5.3.1 p2, 6.5.2.4 p2), so the usual arithmetic conversions of
    /// the operand and the `int` 1. `None` for a pointer.
    fn stepped_at(&mut self, ast: &mut Ast, operand: ExprId) -> Option<TypeId> {
        let integer = integer_of(ast, self.types[operand.index()]?)?;
        let at = self.usual_of(integer, (Rank::Int, Sign::Signed));
        Some(self.integer_type(ast, at))
    }

    /// What C17 6.3.1.1 p2's integer promotions make an integer type: one
    /// whose rank is below `int`'s becomes `int` where `int` holds every
    /// value it does and `unsigned int` otherwise; any other is itself. A
    /// rank and a sign rather than a type, so that [`Checker::evaluate`],
    /// which pushes nothing, can ask it too.
    ///
    /// Mutation: promote nothing; `a_char_is_promoted_before_it_is_added`
    /// overflows.
    fn promoted_of(&self, (rank, sign): (Rank, Sign)) -> (Rank, Sign) {
        match rank {
            Rank::Char | Rank::Short => {
                let from = self.target.integer(rank, sign);
                let int = self.target.integer(Rank::Int, Sign::Signed);
                if int.holds(from.min()) && int.holds(from.max()) {
                    (Rank::Int, Sign::Signed)
                } else {
                    (Rank::Int, Sign::Unsigned)
                }
            }
            Rank::Int | Rank::Long | Rank::LongLong => (rank, sign),
        }
    }

    /// The common type C17 6.3.1.8 p1's usual arithmetic conversions give
    /// two integer types: each promoted, then the same type stays; the
    /// higher rank wins between two of one signedness; an unsigned type of
    /// rank at least the signed one's wins; a signed type that holds every
    /// value of the unsigned one wins; and otherwise the unsigned type of
    /// the signed one's rank, which is where `long` and `unsigned int`
    /// differ between a 64-bit and a 32-bit `long`.
    ///
    /// Mutation: answer the higher rank whatever the sign; `minus_one_is_not_below_one_unsigned`
    /// answers 1.
    fn usual_of(&self, left: (Rank, Sign), right: (Rank, Sign)) -> (Rank, Sign) {
        let (left, right) = (self.promoted_of(left), self.promoted_of(right));
        if left == right {
            return left;
        }
        let signed = |(_, sign): (Rank, Sign)| sign == Sign::Signed;
        if signed(left) == signed(right) {
            return if left.0 >= right.0 { left } else { right };
        }
        let (unsigned, signed_one) = if signed(left) {
            (right, left)
        } else {
            (left, right)
        };
        if unsigned.0 >= signed_one.0 {
            return unsigned;
        }
        let wide = self.target.integer(signed_one.0, signed_one.1);
        let narrow = self.target.integer(unsigned.0, unsigned.1);
        if wide.holds(narrow.max()) {
            signed_one
        } else {
            (signed_one.0, Sign::Unsigned)
        }
    }

    /// The node for an integer type this stage names itself, one per type.
    fn integer_type(&mut self, ast: &mut Ast, (rank, sign): (Rank, Sign)) -> TypeId {
        if (rank, sign) == (Rank::Int, Sign::Signed) {
            return self.int;
        }
        *self
            .integers
            .entry((rank, sign))
            .or_insert_with(|| ast.push_type(Type::Integer { rank, sign }))
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
            // A pointer minus a pointer is `ptrdiff_t` and answered by
            // `Checker::binary` before this (p9). Two pointers added, which
            // p2 forbids, are refused by `binary_operable` before they reach
            // here, and the arm answers nothing for them.
            (true, true) => None,
            // p2 and p3 both allow the pointer on the left.
            (true, false) => Some(lhs),
            // p3 allows it only there: `i - p` is a constraint violation,
            // refused by `binary_operable` before it reaches here, and no type
            // if it did.
            (false, true) if op == BinOp::Sub => None,
            // p2 lets an addition be written the other way round.
            (false, true) => Some(rhs),
            // Two integers are [`Checker::binary`]'s, at their common type;
            // anything else was refused there.
            (false, false) => None,
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
                    Some(pointee_steppable(ast, pointee, true, &|ty| {
                        self.complete_here(ast, ty)
                    }))
                }
                (OperandClass::Arithmetic, OperandClass::Pointer(pointee)) => {
                    Some(pointee_steppable(ast, pointee, false, &|ty| {
                        self.complete_here(ast, ty)
                    }))
                }
                _ => Some(pairing(false)),
            },
            // 6.5.6 p3, which allows the pointer only on the left.
            BinOp::Sub => match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => Some(Ok(())),
                (OperandClass::Pointer(pointee), OperandClass::Arithmetic) => {
                    Some(pointee_steppable(ast, pointee, true, &|ty| {
                        self.complete_here(ast, ty)
                    }))
                }
                (OperandClass::Pointer(left), OperandClass::Pointer(right)) => {
                    // Both pointees are asked: `int[3]` is compatible with
                    // `int[]`, and only one of them is complete. Qualified or
                    // unqualified versions of compatible types (6.5.6 p3), so
                    // a `const int *` and an `int *` subtract.
                    if self.compatible_unqualified(ast, left, right) {
                        Some(
                            pointee_steppable(ast, left, true, &|ty| self.complete_here(ast, ty))
                                .and(pointee_steppable(ast, right, false, &|ty| {
                                    self.complete_here(ast, ty)
                                })),
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
                    self.compatible_unqualified(ast, left, right)
                        && !matches!(ast.ty(left), Type::Function { .. })
                }
                _ => false,
            })),
            // 6.5.9 p2. The `void *` case wants an object type on the other
            // side, which C17 6.2.5 p1 makes every type but a function.
            BinOp::Eq | BinOp::Ne => Some(pairing(match (left, right) {
                (OperandClass::Arithmetic, OperandClass::Arithmetic) => true,
                // A null pointer constant beside any pointer, `(void *)0`
                // included, which is a pointer itself (p2's last item).
                (OperandClass::Pointer(left), OperandClass::Pointer(right)) => {
                    left_is_null
                        || right_is_null
                        || self.compatible_unqualified(ast, left, right)
                        || is_void_beside_an_object(ast, left, right)
                        || is_void_beside_an_object(ast, right, left)
                }
                (OperandClass::Pointer(_), OperandClass::Arithmetic) => right_is_null,
                (OperandClass::Arithmetic, OperandClass::Pointer(_)) => left_is_null,
                _ => false,
            })),
            // 6.5.13 p2 and 6.5.14 p2: each operand a scalar, whatever the
            // other is. Every class named, so that one added later has to
            // say whether it is a scalar rather than passing as "not `void`",
            // which is how a struct once did.
            BinOp::LogAnd | BinOp::LogOr => {
                let scalar = |class: OperandClass| match class {
                    OperandClass::Arithmetic | OperandClass::Pointer(_) => true,
                    OperandClass::Void | OperandClass::Struct => false,
                };
                Some(pairing(scalar(left) && scalar(right)))
            }
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
    /// something reported it, or this compiler left it untyped, as it did
    /// `p - q` until #638 (#1298).
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
            self.is_null_pointer_constant(ast, then),
            self.is_null_pointer_constant(ast, otherwise),
        );

        let paired = match (ast.ty(left), ast.ty(right)) {
            // p5: the usual arithmetic conversions, as for `+`.
            (
                &Type::Integer {
                    rank: left_rank,
                    sign: left_sign,
                },
                &Type::Integer {
                    rank: right_rank,
                    sign: right_sign,
                },
            ) => {
                let common = self.usual_of((left_rank, left_sign), (right_rank, right_sign));
                Some(self.integer_type(ast, common))
            }
            (Type::Void, Type::Void) => Some(left),
            // p6: beside a null pointer constant, `(void *)0` included, the
            // result is the other operand's type, before the rule for a
            // pointer to `void` is asked. Mutation: drop these two arms;
            // `void_pointer_zero_is_a_null_pointer_constant` fails.
            (Type::Pointer(_), Type::Pointer(_)) if nulls.1 => Some(left),
            (Type::Pointer(_), Type::Pointer(_)) if nulls.0 => Some(right),
            (Type::Pointer(left_pointee), Type::Pointer(right_pointee)) => {
                // p3 pairs a pointer to `void` only with a pointer to an
                // object type, which a function is not.
                let left_void = matches!(ast.ty(*left_pointee), Type::Void);
                let right_void = matches!(ast.ty(*right_pointee), Type::Void);
                let left_function = matches!(ast.ty(*left_pointee), Type::Function { .. });
                let right_function = matches!(ast.ty(*right_pointee), Type::Function { .. });
                // p3 and p6: qualified or unqualified versions of compatible
                // types, the result pointing to the one with every qualifier
                // of both, which with `const` the only qualifier is whichever
                // is `const`.
                if self.compatible_unqualified(ast, *left_pointee, *right_pointee) {
                    Some(if ast.is_const(*right_pointee) {
                        right
                    } else {
                        left
                    })
                } else if (left_void && !right_function) || (right_void && !left_function) {
                    // p6: a pointer to `void` qualified with every qualifier
                    // of both, so `1 ? vp : cp` with `const int *cp` is a
                    // `const void *`, not a `void *` the `const` fell out of.
                    // Found by review.
                    let (void, other) = if left_void {
                        (left, *right_pointee)
                    } else {
                        (right, *left_pointee)
                    };
                    let void_pointee = if left_void {
                        *left_pointee
                    } else {
                        *right_pointee
                    };
                    if ast.is_const(other) && !ast.is_const(void_pointee) {
                        let qualified = ast.push_const(void_pointee);
                        Some(ast.push_type(Type::Pointer(qualified)))
                    } else {
                        Some(void)
                    }
                } else {
                    None
                }
            }
            (
                Type::Pointer(_),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) => nulls.1.then_some(left),
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
                Type::Pointer(_),
            ) => nulls.0.then_some(right),
            // `decayed` answers neither an array nor a function; each would
            // be the pointer it decays to if it did.
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Pointer(_),
                Type::Void,
            )
            | (
                Type::Void,
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Pointer(_),
            )
            | (Type::Array { .. } | Type::Function { .. }, _)
            | (_, Type::Array { .. } | Type::Function { .. }) => None,
            // p3: both of one structure type, which two structs are when
            // they are one tag, and nothing else beside a struct.
            // After lvalue conversion (6.3.2.1 p2), which drops a top-level
            // qualifier, so `const struct S` beside `struct S` is one type.
            (Type::Struct { .. }, Type::Struct { .. }) => self
                .compatible_unqualified(ast, left, right)
                .then_some(left),
            (Type::Struct { .. }, _) | (_, Type::Struct { .. }) => None,
            // No operator takes one, and a list chosen by `?:` would be a
            // copy of one, which only `__builtin_va_copy` makes (#1214).
            (Type::VaList, _) | (_, Type::VaList) => None,
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
    /// whichever side it is on, and the note says why. So is a struct on the
    /// left, so `s * 1` points at `s`.
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
            // A struct, like `void`, is an operand of no operator here, so
            // it is wrong alone whatever is beside it.
            Refused::Pairing => match ast.ty(left) {
                Type::Void | Type::Struct { .. } | Type::VaList => true,
                Type::Pointer(_) => !takes_a_pointer(op),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Array { .. }
                | Type::Function { .. } => false,
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
    /// [`Checker::conditional`] ask. `assignable` answers an array source as
    /// this pointer without building it, since its callers hold the tree
    /// immutably (#621).
    fn decayed(&mut self, ast: &mut Ast, ty: TypeId) -> TypeId {
        match ast.ty(ty) {
            Type::Array { element, .. } => {
                let element = *element;
                ast.push_type(Type::Pointer(element))
            }
            Type::Function { .. } => ast.push_type(Type::Pointer(ty)),
            Type::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            }
            | Type::Void
            | Type::VaList
            | Type::Pointer(_)
            | Type::Struct { .. } => ty,
        }
    }

    /// `__builtin_va_start`, `__builtin_va_end` or `__builtin_va_copy`,
    /// which return nothing (C17 7.16.1.2 to 7.16.1.4), and whose operands
    /// are lists, and for `__builtin_va_start` the function's last parameter
    /// after its list. See ADR-0126.
    fn va_op(
        &mut self,
        ast: &mut Ast,
        op: VaOp,
        operands: &[ExprId],
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let lists = match op {
            VaOp::Start | VaOp::End => &operands[..1],
            VaOp::Copy => operands,
        };
        let mut fine = true;
        for &list in lists {
            fine &= self.va_list_operand(ast, op.as_str(), list, diagnostics);
        }
        // **The last parameter of the variadic function it is in**, which
        // 7.16.1.4 p4 makes undefined to get wrong and which is known where
        // it is written. A parameter is in scope only in its own function's
        // body, so a name resolving to the last parameter of a variadic
        // definition is that function's. Mutation: accept any name;
        // `a_va_start_names_the_last_parameter_of_its_variadic_function`'s
        // `not_the_last` builds.
        if op == VaOp::Start {
            let last = operands[1];
            let named = matches!(ast.expr(last), Expr::Identifier { .. })
                .then(|| self.resolution.resolved(last))
                .flatten()
                .map(|binding| self.resolution.binding(binding).name);
            let is_last = named.is_some_and(|name| {
                ast.items().iter().any(|item| match item {
                    Item::Function(function) => matches!(
                        ast.ty(function.ty),
                        Type::Function {
                            parameters: Parameters::Prototype(parameters),
                            variadic: true,
                            ..
                        } if parameters.last().and_then(|last| last.name) == Some(name)
                    ),
                    Item::Declaration { .. } | Item::Error { .. } => false,
                })
            });
            if !is_last {
                fine = false;
                diagnostics.report(
                    Diagnostic::error(
                        "`__builtin_va_start` names something other than the last parameter of a variadic function it is in",
                    )
                    .with_code(OPERANDS)
                    .with_label(Label::primary(ast.expr(last).span(), "this"))
                    .with_label(Label::secondary(span, "in this"))
                    .with_note("C17 7.16.1.4 p4 makes anything but the last parameter before the `...` undefined"),
                );
            }
        }
        fine.then(|| ast.push_type(Type::Void))
    }

    /// `__builtin_va_arg(list, ty)`, of type `ty`: a complete object type the
    /// default argument promotions leave as it is, since the argument
    /// arrived promoted and C17 7.16.1.1 p2 makes reading it as another type
    /// undefined. See ADR-0126.
    fn va_arg(
        &mut self,
        ast: &mut Ast,
        list: ExprId,
        ty: TypeId,
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        let fine = self.va_list_operand(ast, "__builtin_va_arg", list, diagnostics);
        // Mutation: drop the promotion test; `a_va_arg_of_a_char_is_refused`
        // builds.
        let why = if ast.is_promoted_by_default(ty) {
            Some("the default argument promotions make it an `int`")
        } else {
            match ast.ty(ty) {
                Type::Integer { .. } | Type::Pointer(_) => None,
                Type::Void | Type::Function { .. } | Type::Array { .. } => {
                    Some("it is no complete object type an argument can have")
                }
                Type::Struct { .. } => Some("a struct passed past the parameters is not read yet"),
                Type::VaList => Some("a list is passed as no argument here"),
            }
        };
        if let Some(why) = why {
            let spelled = self.spelled(ast, ty);
            diagnostics.report(
                Diagnostic::error(format!("`__builtin_va_arg` cannot read a `{spelled}`"))
                    .with_code(OPERANDS)
                    .with_label(Label::primary(span, why))
                    .with_note("C17 7.16.1.1 p2 makes reading an argument as a type it was not passed as undefined"),
            );
            return None;
        }
        fine.then_some(ty)
    }

    /// Whether `list`, an operand of `builtin`, is an lvalue of type
    /// `__builtin_va_list`, reporting it where it is not.
    fn va_list_operand(
        &mut self,
        ast: &Ast,
        builtin: &str,
        list: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        let Some(ty) = self.types[list.index()] else {
            return false;
        };
        if matches!(ast.ty(ty), Type::VaList)
            && matches!(self.designates(ast, list), Designates::Object)
        {
            return true;
        }
        let spelled = self.spelled(ast, ty);
        diagnostics.report(
            Diagnostic::error(format!("`{builtin}` cannot take `{spelled}`"))
                .with_code(OPERANDS)
                .with_label(Label::primary(
                    ast.expr(list).span(),
                    format!("this is `{spelled}`"),
                ))
                .with_note("C17 7.16.1 p1 asks a `va_list` object of each of these"),
        );
        false
    }

    /// What `id` designates, from its shape and its type.
    ///
    /// An identifier designates what its declaration declared (6.5.1 p2); `*e`
    /// designates a function or an object, but not one of type `void`, which
    /// is no lvalue (6.3.2.1 p1); a subscript and `->` designate an object;
    /// `.` designates one where its base does (6.5.2.3 p3, p4). Everything
    /// else is a value: a comma, a conditional, an assignment (6.5.16 p3),
    /// a call, a constant and every arithmetic operator. Exhaustive over
    /// `Expr`, so a kind added later has to say which it is.
    ///
    /// A chain of `.` is walked in a loop rather than by recursion, since
    /// nothing bounds its length but the source and every other walk over
    /// it here is already a loop; ten thousand of them overflowed the stack.
    /// A `.` with no type of its own was refused where it was typed, and is
    /// `Unknown` rather than whatever its base designates: `h.m` is not a
    /// function because `h` is one.
    fn designates(&self, ast: &Ast, id: ExprId) -> Designates {
        let typed = |id: ExprId| self.types[id.index()].map(|ty| ast.ty(ty));
        let mut id = id;
        while let Expr::Member {
            arrow: false, base, ..
        } = ast.expr(id)
        {
            if typed(id).is_none() {
                return Designates::Unknown;
            }
            id = *base;
        }
        match ast.expr(id) {
            Expr::Identifier { span: _ } => match typed(id) {
                None => Designates::Unknown,
                Some(Type::Function { .. }) => Designates::Function,
                Some(_) => Designates::Object,
            },
            // C17 6.5.1 p4: a string literal is an lvalue.
            Expr::String { .. } => match typed(id) {
                None => Designates::Unknown,
                Some(_) => Designates::Object,
            },
            Expr::Unary {
                op: UnOp::Deref, ..
            } => match typed(id) {
                None => Designates::Unknown,
                Some(Type::Function { .. }) => Designates::Function,
                Some(Type::Void) => Designates::Value,
                Some(_) => Designates::Object,
            },
            Expr::Subscript {
                base: _,
                index: _,
                span: _,
            }
            | Expr::Member {
                arrow: true,
                base: _,
                member: _,
                span: _,
            } => match typed(id) {
                None => Designates::Unknown,
                Some(_) => Designates::Object,
            },
            Expr::Member {
                arrow: false,
                base: _,
                member: _,
                span: _,
            } => unreachable!("the loop above walked every `.` down to its base"),
            Expr::Unary { .. }
            | Expr::Number { span: _ }
            | Expr::Length { value: _, span: _ }
            | Expr::Enumerator { .. }
            | Expr::Character { span: _ }
            | Expr::Binary { .. }
            | Expr::Assign { .. }
            | Expr::Comma { .. }
            | Expr::Conditional { .. }
            | Expr::SizeofType { .. }
            | Expr::SizeofExpr { .. }
            | Expr::Cast { .. }
            | Expr::Call { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. } => Designates::Value,
            Expr::Error { span: _ } => Designates::Unknown,
        }
    }

    /// Report `operand` under [`UNWRITABLE`] if it is not a modifiable
    /// lvalue, which `what` (the left operand of `=`, the operand of `++`)
    /// has to be by `clause`, and answer whether it was reported.
    ///
    /// A modifiable lvalue is an lvalue that is not an array, not of an
    /// incomplete type and not `const`-qualified (C17 6.3.2.1 p1). A struct
    /// not complete where it is written is `SC0315`'s, which says it cannot
    /// be modified, and is not reported twice.
    fn refuse_unwritable(
        &self,
        ast: &Ast,
        operand: ExprId,
        what: &str,
        clause: &str,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        let label = match self.designates(ast, operand) {
            Designates::Unknown => return false,
            Designates::Object => {
                let Some(ty) = self.types[operand.index()] else {
                    return false;
                };
                if ast.is_const(ty) {
                    format!("this is `{}`", self.spelled(ast, ty))
                } else if self.has_const_member(ast, ty) {
                    // C17 6.3.2.1 p1: not a structure "with any member
                    // (including, recursively, any member or element of all
                    // contained aggregates or unions) with a const-qualified
                    // type". Found by review.
                    format!(
                        "this is `{}`, which has a `const` member",
                        self.spelled(ast, ty)
                    )
                } else if matches!(ast.ty(ty), Type::Array { .. }) {
                    format!("this is `{}`, an array", self.spelled(ast, ty))
                } else {
                    return false;
                }
            }
            Designates::Function => "this is a function".to_owned(),
            Designates::Value => self.not_an_object(ast, operand),
        };
        diagnostics.report(
            Diagnostic::error(format!("{what} cannot be written to"))
                .with_code(UNWRITABLE)
                .with_label(Label::primary(ast.expr(operand).span(), label))
                .with_note(format!(
                    "{clause} wants a modifiable lvalue: an object that is not an array, not of an incomplete type and not `const` (C17 6.3.2.1 p1)"
                )),
        );
        true
    }

    /// Whether `ty` is a structure with a `const` member, through every
    /// member that is a structure or an array of them, as C17 6.3.2.1 p1 asks.
    fn has_const_member(&self, ast: &Ast, ty: TypeId) -> bool {
        let Type::Struct { .. } = ast.ty(ty) else {
            return false;
        };
        let Some(definition) = self
            .resolution
            .definition(self.resolution.tag(self.resolution.bound(ast, ty)))
        else {
            return false;
        };
        let Type::Struct {
            members: Some(members),
            ..
        } = ast.ty(definition)
        else {
            return false;
        };
        members.iter().any(|member| {
            let mut ty = member.ty;
            while let Type::Array { element, .. } = ast.ty(ty) {
                if ast.is_const(ty) {
                    return true;
                }
                ty = *element;
            }
            ast.is_const(ty) || self.has_const_member(ast, ty)
        })
    }

    /// Report `operand` under [`UNWRITABLE`] if `&` cannot take its
    /// address: C17 6.5.3.2 p1 takes a function designator, the result of
    /// `[]` or unary `*`, or an lvalue designating an object. The result of
    /// `*` is taken whatever it is, so `&*v` with `v` a `void *` stays
    /// valid, as p3 has neither operator evaluated. The result of `[]` needs
    /// no case of its own, since [`Checker::designates`] answers an object
    /// for every subscript. Answers whether it was reported.
    fn refuse_unaddressable(
        &self,
        ast: &Ast,
        operand: ExprId,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if matches!(
            ast.expr(operand),
            Expr::Unary {
                op: UnOp::Deref,
                ..
            }
        ) || self.designates(ast, operand) != Designates::Value
        {
            return false;
        }
        diagnostics.report(
            Diagnostic::error("`&` needs an object or a function to take the address of")
                .with_code(UNWRITABLE)
                .with_label(Label::primary(
                    ast.expr(operand).span(),
                    self.not_an_object(ast, operand),
                ))
                .with_note(
                    "C17 6.5.3.2 p1 takes a function designator, the result of `[]` or unary `*`, or an lvalue designating an object",
                ),
        );
        true
    }

    /// The label for an operand that designates nothing: a `void` reached
    /// through `*`, which 6.3.2.1 p1 makes no lvalue, or any other value.
    fn not_an_object(&self, ast: &Ast, operand: ExprId) -> String {
        match self.types[operand.index()].map(|ty| ast.ty(ty)) {
            Some(Type::Void) => "this is `void`, not an object".to_owned(),
            _ => "this is a value, not an object".to_owned(),
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
            (
                Type::Pointer(pointee),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) if matches!(op, BinOp::Add | BinOp::Sub) => {
                unsteppable(ast, *pointee, &|ty| self.complete_here(ast, ty))
            }
            _ => None,
        };

        let target_spelled = self.spelled(ast, target);
        let source_spelled = self.spelled(ast, source);
        let (primary, secondary) = if !matches!(
            ast.ty(source),
            Type::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _
            }
        ) {
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
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) => Some(true),
            (
                Type::Pointer(pointee),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) => match op {
                BinOp::Add | BinOp::Sub => {
                    Some(unsteppable(ast, *pointee, &|ty| self.complete_here(ast, ty)).is_none())
                }
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
            // p1 and p2 want arithmetic or pointer operands, which a struct
            // is neither.
            (Type::Struct { .. } | Type::VaList, _) | (_, Type::Struct { .. } | Type::VaList) => {
                Some(false)
            }
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Pointer(_)
                | Type::Void,
                Type::Pointer(_) | Type::Void | Type::Array { .. } | Type::Function { .. },
            )
            | (
                Type::Void,
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) => Some(false),
        }
    }

    /// A condition that is not a scalar, refused under `OPERANDS` in the
    /// words `!` and `&&` use, and whether it was. `word` is what read it:
    /// `if`, `while`, `for` or `?:`.
    ///
    /// `void` and a struct are refused, which is right only because every
    /// other type this compiler has is a scalar or becomes one: an array or a
    /// function is a pointer by 6.3.2.1 p3 and p4 before C reads it as a
    /// condition. Nothing here converts it; it is neither, so it passes.
    fn refuse_a_void_condition(
        &self,
        ast: &Ast,
        word: &str,
        condition: ExprId,
        ty: TypeId,
        diagnostics: &mut DiagnosticSink,
    ) -> bool {
        if !matches!(ast.ty(ty), Type::Void | Type::Struct { .. }) {
            return false;
        }
        let spelled = self.spelled(ast, ty);
        diagnostics.report(
            Diagnostic::error(format!("`{word}` cannot take `{spelled}`"))
                .with_code(OPERANDS)
                .with_label(Label::primary(
                    ast.expr(condition).span(),
                    format!("this is `{spelled}`"),
                )),
        );
        true
    }

    /// A `switch` on a value that is not an integer, refused under `OPERANDS`
    /// in the words a condition is: C17 6.8.4.2 p1 wants integer type, and a
    /// pointer, which a condition takes, is not one.
    fn refuse_a_switch_on_no_integer(
        &self,
        ast: &Ast,
        condition: ExprId,
        ty: TypeId,
        diagnostics: &mut DiagnosticSink,
    ) {
        match ast.ty(ty) {
            Type::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            } => return,
            Type::Void
            | Type::VaList
            | Type::Pointer(_)
            | Type::Array { .. }
            | Type::Function { .. }
            | Type::Struct { .. } => {}
        }
        let spelled = self.spelled(ast, ty);
        diagnostics.report(
            Diagnostic::error(format!("`switch` cannot take `{spelled}`"))
                .with_code(OPERANDS)
                .with_label(Label::primary(
                    ast.expr(condition).span(),
                    format!("this is `{spelled}`"),
                ))
                .with_note("C17 6.8.4.2 p1 wants an expression of integer type"),
        );
    }

    /// Hold each `switch`'s labels to C17 6.8.4.2 p3: a `case` value is an
    /// integer constant expression, no two have one value, and there is at
    /// most one `default`.
    ///
    /// After every expression has its value, which is what the first two
    /// ask. Each value is converted to the promoted controlling type before
    /// two are compared (p5), which is also the type recorded for the
    /// controlling expression in [`Types::controlling`] and
    /// [`Types::operated`].
    fn check_switches(&mut self, ast: &mut Ast, diagnostics: &mut DiagnosticSink) {
        for id in ast.stmt_ids().collect::<Vec<_>>() {
            let Stmt::Switch { condition, .. } = *ast.stmt(id) else {
                continue;
            };
            // p5: the controlling expression is promoted, and each `case`
            // value converted to the promoted type before it is compared, so
            // two values that convert to one are the same `case`.
            let controlling = self.types[condition.index()]
                .and_then(|ty| integer_of(ast, ty))
                .map(|integer| self.promoted_of(integer));
            if let Some(controlling) = controlling {
                let ty = self.integer_type(ast, controlling);
                self.operated[condition.index()].get_or_insert(ty);
                self.controlling.insert(condition, ty);
            }
            let convert = |number: i128| {
                controlling.map_or(number, |(rank, sign)| {
                    self.target.integer(rank, sign).convert(number)
                })
            };
            let mut seen: HashMap<i128, Span> = HashMap::new();
            let mut default: Option<Span> = None;
            for &label in self.resolution.labels_of_switch(id) {
                match ast.stmt(label) {
                    Stmt::Case { value, .. } => {
                        let at = ast.expr(*value).span();
                        let Some(number) = self.values[value.index()] else {
                            // A value nothing typed was reported already. One
                            // shaped as a constant whose evaluation C leaves
                            // undefined is a constant expression with no value,
                            // and is said to be that, as an array's length is.
                            // Found by review.
                            if self.undefined[value.index()] {
                                diagnostics.report(
                                    Diagnostic::error("a `case` value has no defined value")
                                        .with_code(NOT_CONSTANT)
                                        .with_label(Label::primary(
                                            at,
                                            "this overflows its type, divides by zero, or shifts too far",
                                        ))
                                        .with_note(
                                            "a constant expression evaluates to a value its type \
                                             can hold (C17 6.6 p4)",
                                        ),
                                );
                            } else if self.types[value.index()].is_some() {
                                diagnostics.report(
                                    Diagnostic::error(
                                        "a `case` value is not an integer constant expression",
                                    )
                                    .with_code(NOT_CONSTANT)
                                    .with_label(Label::primary(at, "not a constant expression"))
                                    .with_note("C17 6.8.4.2 p3"),
                                );
                            }
                            continue;
                        };
                        let number = convert(number);
                        match seen.get(&number) {
                            Some(&first) => diagnostics.report(
                                Diagnostic::error("duplicate case value")
                                    .with_code(SWITCH_LABELS)
                                    .with_label(Label::primary(at, "this value again"))
                                    .with_label(Label::secondary(first, "first here"))
                                    .with_note("C17 6.8.4.2 p3"),
                            ),
                            None => {
                                seen.insert(number, at);
                            }
                        }
                    }
                    Stmt::Default { span, .. } => {
                        let at = Span::new(span.file(), span.start(), span.start() + 7);
                        match default {
                            Some(first) => diagnostics.report(
                                Diagnostic::error("multiple default labels in one switch")
                                    .with_code(SWITCH_LABELS)
                                    .with_label(Label::primary(at, "a second `default`"))
                                    .with_label(Label::secondary(first, "the first"))
                                    .with_note("C17 6.8.4.2 p3"),
                            ),
                            None => default = Some(at),
                        }
                    }
                    _ => unreachable!("name resolution lists only `case` and `default`"),
                }
            }
        }
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
            Receiving::Switch => {
                self.refuse_a_switch_on_no_integer(ast, value, source, diagnostics);
                return;
            }
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
            Receiving::Condition { .. } | Receiving::Switch => return,
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
            variadic,
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
        // 6.5.2.2 p1 once more: what a call returns is complete where the
        // call is. Reported, and the call still typed, since its type is the
        // one declared and what follows is asked of that.
        if matches!(ast.ty(returns), Type::Struct { .. }) && !self.complete_here(ast, returns) {
            let spelled = self.spelled(ast, returns);
            diagnostics.report(
                Diagnostic::error(format!("this call returns incomplete type `{spelled}`"))
                    .with_code(INCOMPLETE_OBJECT)
                    .with_label(Label::primary(ast.expr(call).span(), "this call"))
                    .with_note(
                        "a function called returns `void` or a complete type (C17 6.5.2.2 p1)",
                    ),
            );
        }

        let Parameters::Prototype(parameters) = parameters else {
            // With no prototype nothing is known of what the arguments should
            // be, but a `void` value is no argument under any: C17 6.3.2.2
            // lets a `void` expression's value be used in no way at all.
            // Unasked, `f(g())` with `g` returning `void` reached the lowering
            // and panicked there. A prototype's parameters ask the same of each
            // argument in `check_argument`, through `assignable`.
            for &argument in arguments {
                let Some(ty) = self.types[argument.index()] else {
                    continue;
                };
                if !matches!(ast.ty(ty), Type::Void) {
                    continue;
                }
                diagnostics.report(
                    Diagnostic::error("cannot pass `void` as an argument")
                        .with_code(MISMATCH)
                        .with_label(Label::primary(ast.expr(argument).span(), "this is `void`"))
                        .with_note(
                            "a `void` expression has no value, and its value cannot be used (C17 6.3.2.2)",
                        ),
                );
            }
            return Some(returns);
        };

        // A `match` on the ordering rather than two comparisons: the worst
        // defect this project has had is a gate written as two of those with a
        // case answered by neither.
        let (what, expected, found) = match arguments.len().cmp(&parameters.len()) {
            Ordering::Less => ("too few", parameters.len(), arguments.len()),
            // **Past the parameters of a variadic prototype**, each further
            // argument takes no declared type: it is passed after the default
            // argument promotions (C17 6.5.2.2 p7), which the lowering applies,
            // and only a `void` one is no argument, as under `()` (#1213).
            Ordering::Greater if *variadic => {
                for (&argument, parameter) in arguments.iter().zip(parameters) {
                    let declared = parameter.name.unwrap_or(parameter.span);
                    self.check_argument(ast, argument, parameter.ty, declared, diagnostics);
                }
                for &argument in &arguments[parameters.len()..] {
                    if self.types[argument.index()]
                        .is_some_and(|ty| matches!(ast.ty(ty), Type::Void))
                    {
                        diagnostics.report(
                            Diagnostic::error("cannot pass `void` as an argument")
                                .with_code(MISMATCH)
                                .with_label(Label::primary(ast.expr(argument).span(), "this is `void`"))
                                .with_note(
                                    "a `void` expression has no value, and its value cannot be used (C17 6.3.2.2)",
                                ),
                        );
                    }
                }
                return Some(returns);
            }
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
            let declared = self.resolution.standing(binding);
            diagnostic = diagnostic.with_label(Label::secondary(
                self.resolution.binding(declared).name,
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
        let null = self.is_null_pointer_constant(ast, argument);
        // C17 6.5.2.2 p2 holds an argument to what may be assigned to its
        // parameter, and nothing may be assigned to `void` (6.5.16.1 p1).
        // `assignable` leaves a `void` target unanswered, for the reason its
        // comment gives, so a `void` parameter is asked here; a declaration
        // may still have one (6.7.6.3 p12). Before this, `f(1)` against
        // `int f(void b);` built, and `f(g())` with `g` returning `void`
        // reached the lowering and panicked. A `void` argument to any other
        // parameter is `assignable`'s own `false`.
        let refused = matches!(ast.ty(parameter), Type::Void)
            || self.assignable(ast, parameter, source, null) == Some(false);
        if !refused {
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

    /// Whether two types are compatible, C17 6.2.7 p1: `Ast::compatible`'s
    /// answer, and **two arrays of constant lengths only where the lengths
    /// are equal** (6.7.6.2 p6), which that comparison cannot ask since the
    /// values are this stage's. `int (*pa)[3] = &a;` with `a` an `int[4]`
    /// built once the lowering read arrays, and `clang` refuses it (#621).
    /// A length with no value, a variable one or none written, is compatible
    /// with any, as p6 says.
    ///
    /// **With the qualifiers of the two types themselves set aside**, which
    /// is what every question this stage asks wants: an assignment takes the
    /// left operand's unqualified type (6.5.16.1 p1), and the operators and
    /// `?:` take qualified or unqualified versions of compatible types
    /// (6.5.6 p3, 6.5.8 p2, 6.5.9 p2, 6.5.15 p3). What is pointed to keeps its
    /// qualifiers in the comparison, so `const int *` is not `int *`.
    fn compatible_unqualified(&self, ast: &Ast, left: TypeId, right: TypeId) -> bool {
        ast.compatible_unqualified(left, right, &|a, b| self.resolution.same_tag(a, b))
            && self.lengths_agree(ast, left, right)
    }

    /// Whether every pair of array lengths two compatible types hold at the
    /// same place agrees, where both have a value. Through pointers and
    /// array elements, which are where one array type nests in another; a
    /// function's parameters are adjusted to pointers (6.7.6.3 p7), and the
    /// length an adjustment drops is not part of the type. Bounded by the
    /// parser's `MAX_NESTING`, as `Ast::compatible` is.
    fn lengths_agree(&self, ast: &Ast, left: TypeId, right: TypeId) -> bool {
        match (ast.ty(left), ast.ty(right)) {
            (Type::Pointer(left), Type::Pointer(right)) => self.lengths_agree(ast, *left, *right),
            (
                Type::Array {
                    element: left,
                    length: left_length,
                    ..
                },
                Type::Array {
                    element: right,
                    length: right_length,
                    ..
                },
            ) => {
                let values = (
                    left_length.and_then(|length| self.values[length.index()]),
                    right_length.and_then(|length| self.values[length.index()]),
                );
                let equal = match values {
                    (Some(left), Some(right)) => left == right,
                    _ => true,
                };
                equal && self.lengths_agree(ast, *left, *right)
            }
            // Nothing else nests an array type that `Ast::compatible` has
            // not already compared: written out so a kind added later says
            // whether it does.
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Void
                | Type::VaList
                | Type::Pointer(_)
                | Type::Array { .. }
                | Type::Function { .. }
                | Type::Struct { .. },
                _,
            ) => true,
        }
    }

    /// Whether a value of type `source` may be assigned to a place of type
    /// `target`, C17 6.5.16.1 p1.
    ///
    /// `None` where this stage does not answer: a `void` target, an array or a
    /// function target, is a different constraint, and 6.5.16 p2's
    /// requirement that the place be a modifiable lvalue is another. An array
    /// source and a function source are answered as the pointers they convert
    /// to.
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
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) => Some(true),
            // p1's pointer case, with the `void *` half of it, and the
            // qualifier rule both halves share: what the target points to has
            // every qualifier of what the source points to, so a conversion
            // never discards `const`. The pointers' own qualifiers are not
            // asked: p1 takes the left operand's unqualified type.
            //
            // **The `void *` half is for a pointer to an object only**: a
            // function is no object type (6.2.5 p1), so a pointer to one and
            // a `void *` convert only by a cast. Accepted, `void *v = f;` and
            // `int (*g)(int) = v;` built in silence, and `g` was called as
            // whatever `v` held (#1212).
            (Type::Pointer(target_pointee), Type::Pointer(source_pointee)) => {
                let function = |pointee: TypeId| matches!(ast.ty(pointee), Type::Function { .. });
                Some(
                    (!ast.is_const(*source_pointee) || ast.is_const(*target_pointee))
                        && (self.compatible_unqualified(ast, *target_pointee, *source_pointee)
                            || (matches!(ast.ty(*target_pointee), Type::Void)
                                && !function(*source_pointee))
                            || (matches!(ast.ty(*source_pointee), Type::Void)
                                && !function(*target_pointee))),
                )
            }
            // 6.3.2.3 p3: only a null pointer constant may be assigned to a
            // pointer from an integer.
            (
                Type::Pointer(_),
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
            ) => Some(source_is_null),
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                },
                Type::Pointer(_),
            ) => Some(false),
            // 6.5.16.1 p1 lists what may be assigned to an object, and a
            // `void` expression has no value to be any of it; 6.8.6.4 p3 and
            // 6.7.9 p11 hold a returned value and an initializer to the same
            // list. Answering `None` here let `return h();` from a function
            // returning `int` build in silence.
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Pointer(_),
                Type::Void,
            ) => Some(false),
            // **An array source is the pointer to its first element it has
            // become** (6.3.2.1 p3) before anything is assigned, so p1 asks
            // that pointer: `int **pp = a;` with `a` an `int[2]` is a pointer
            // to `int` assigned to a pointer to `int *`, and `int x = a;` a
            // pointer assigned to an integer. Left unanswered, both built
            // once the lowering read arrays, and the first dereferenced an
            // `int` as a pointer with nothing said (#621).
            (Type::Pointer(target_pointee), Type::Array { element, .. }) => Some(
                (!ast.is_const(*element) || ast.is_const(*target_pointee))
                    && (self.compatible_unqualified(ast, *target_pointee, *element)
                        || matches!(ast.ty(*target_pointee), Type::Void)),
            ),
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Struct { .. },
                Type::Array { .. },
            ) => Some(false),
            // **A function source is the pointer to it it has become**
            // (6.3.2.1 p4), as an array source is above, so p1 asks that
            // pointer: a function of another type assigned to a function
            // pointer is a call through the wrong type later, and a function
            // assigned to an integer or a struct is no assignment p1 lists.
            // Left unanswered, `int (*g)(int *) = h;` with `h` taking `int`
            // built in silence (#1212).
            (Type::Pointer(target_pointee), Type::Function { .. }) => {
                Some(self.compatible_unqualified(ast, *target_pointee, source))
            }
            (
                Type::Integer {
                    rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                    sign: _,
                }
                | Type::Struct { .. },
                Type::Function { .. },
            ) => Some(false),
            (Type::Void | Type::Array { .. } | Type::Function { .. }, _) => None,
            // **A list is filled only by `__builtin_va_start` or
            // `__builtin_va_copy`** (C17 7.16.1 p1): on some targets it is an
            // array, which nothing assigns, and on the others a copy made by
            // `=` is no list `va_arg` may read. So none is assigned,
            // initialized, passed or returned, as `clang` lets one be where
            // it is a pointer (#1214).
            (Type::VaList, _) | (_, Type::VaList) => Some(false),
            // p1: a structure type compatible with the target's, which two
            // structs are when they are one tag, and nothing else.
            (Type::Struct { .. }, Type::Struct { .. }) => {
                Some(self.compatible_unqualified(ast, target, source))
            }
            (Type::Struct { .. }, _) | (_, Type::Struct { .. }) => Some(false),
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
    fn is_null_pointer_constant(&self, ast: &Ast, value: ExprId) -> bool {
        self.values[value.index()] == Some(0) || self.void_null(ast, value)
    }

    /// Whether `id` is an integer constant expression of value 0 cast to
    /// `void *`, the second spelling of a null pointer constant (C17 6.3.2.3
    /// p3) and how `NULL` is commonly defined. Mutation: answer `false`;
    /// `void_pointer_zero_is_a_null_pointer_constant` fails.
    fn void_null(&self, ast: &Ast, id: ExprId) -> bool {
        let Expr::Cast { ty, operand, .. } = *ast.expr(id) else {
            return false;
        };
        matches!(ast.ty(ty), Type::Pointer(pointee)
            if matches!(ast.ty(*pointee), Type::Void) && !ast.is_const(*pointee))
            && self.values[operand.index()] == Some(0)
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
    /// Computed at the type the operation is performed at, and given no
    /// value where C gives the operation no defined result: a signed result
    /// outside that type (6.6 p4 for a constant, 6.5 p5 at run time), a division
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
        // **At the type the operation is performed at** (6.3.1.8): the
        // common type for a comparison, the result's for anything else. An
        // operand is converted to it first, so `-1 < 1u` compares 4294967295
        // with 1, and a result is held to it: reduced modulo its range where
        // it is unsigned (6.2.5 p9), and no value where a signed one does not
        // hold it (6.5 p5).
        let range =
            |ty: TypeId| integer_of(ast, ty).map(|(rank, sign)| self.target.integer(rank, sign));
        let at = self.operated[id.index()]
            .or(self.types[id.index()])
            .and_then(range);
        let held = |result: i128| {
            let at = at?;
            if at.signed() {
                at.holds(result).then_some(result)
            } else {
                Some(at.convert(result))
            }
        };
        let converted = |id: ExprId| {
            let value = value(id)?;
            Some(at.map_or(value, |at| at.convert(value)))
        };
        match *ast.expr(id) {
            Expr::Unary { op, operand, .. } => match op {
                UnOp::Plus => converted(operand),
                UnOp::Minus => held(-converted(operand)?),
                UnOp::BitNot => held(!converted(operand)?),
                // p5 compares the operand itself with zero.
                UnOp::Not => Some(i128::from(value(operand)? == 0)),
                UnOp::Deref
                | UnOp::AddrOf
                | UnOp::PreInc
                | UnOp::PreDec
                | UnOp::PostInc
                | UnOp::PostDec => None,
            },
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
                    return self.unevaluated_constant(ast, rhs).then_some(answer);
                }
                let truth = |holds: bool| Some(i128::from(holds));
                if let BinOp::LogAnd | BinOp::LogOr = op {
                    let right = value(rhs)?;
                    return match op {
                        BinOp::LogAnd => truth(left != 0 && right != 0),
                        _ => truth(left != 0 || right != 0),
                    };
                }
                let width = i128::from(at?.bits());
                let signed = at?.signed();
                // A shift's right operand is promoted on its own (6.5.7 p3)
                // and is a count, not converted to the left's type.
                if let BinOp::Shl | BinOp::Shr = op {
                    let (lhs, rhs) = (converted(lhs)?, value(rhs)?);
                    if !(0..width).contains(&rhs) {
                        return None;
                    }
                    return match op {
                        // 6.5.7 p4: a signed left shift of a negative value,
                        // or one whose result the type does not hold, has
                        // none; an unsigned one is reduced.
                        BinOp::Shl if signed && lhs < 0 => None,
                        BinOp::Shl => held(lhs << rhs),
                        // Arithmetic for a signed value, as the backend's
                        // `ashr` is; an unsigned one is never negative here.
                        _ => Some(lhs >> rhs),
                    };
                }
                let (lhs, rhs) = (converted(lhs)?, converted(rhs)?);
                match op {
                    // Two `unsigned long long`s near 2^64 multiply past
                    // `i128`; wrapping keeps the low 128 bits, of which an
                    // unsigned result keeps the low 64, and a product of two
                    // signed values of 64 bits or fewer never wraps.
                    BinOp::Mul => held(lhs.wrapping_mul(rhs)),
                    BinOp::Div => (rhs != 0).then(|| lhs / rhs).and_then(held),
                    // 6.5.5 p6 defines `a % b` only where `a / b` is
                    // representable, so `INT_MIN % -1` has no value though
                    // its remainder, zero, would fit.
                    BinOp::Rem => (rhs != 0)
                        .then(|| lhs % rhs)
                        .and_then(|r| held(lhs / rhs).map(|_| r)),
                    BinOp::Add => held(lhs + rhs),
                    BinOp::Sub => held(lhs - rhs),
                    BinOp::Lt => truth(lhs < rhs),
                    BinOp::Gt => truth(lhs > rhs),
                    BinOp::Le => truth(lhs <= rhs),
                    BinOp::Ge => truth(lhs >= rhs),
                    BinOp::Eq => truth(lhs == rhs),
                    BinOp::Ne => truth(lhs != rhs),
                    BinOp::BitAnd => held(lhs & rhs),
                    BinOp::BitXor => held(lhs ^ rhs),
                    BinOp::BitOr => held(lhs | rhs),
                    BinOp::Shl | BinOp::Shr | BinOp::LogAnd | BinOp::LogOr => {
                        unreachable!("answered above")
                    }
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
                if !self.unevaluated_constant(ast, skipped) {
                    return None;
                }
                converted(taken)
            }
            // Counted, and so known.
            Expr::Length { value, .. } => Some(i128::from(value)),
            Expr::Enumerator { enumerator, .. } => self.enumerator_values[enumerator.index()],
            // To an integer type, the operand's value converted to it (6.3.1.3);
            // a pointer has no value here, and `(int *)0` is lowered as the
            // null it is.
            Expr::Cast { operand, .. } => {
                at?;
                converted(operand)
            }
            // Set when it was typed, which a `sizeof` that has a type always is.
            Expr::SizeofType { .. } | Expr::SizeofExpr { .. } => None,
            Expr::Number { .. }
            | Expr::Character { .. }
            | Expr::String { .. }
            | Expr::Identifier { .. }
            | Expr::Assign { .. }
            | Expr::Comma { .. }
            | Expr::Subscript { .. }
            | Expr::Member { .. }
            | Expr::Call { .. }
            | Expr::VaOp { .. }
            | Expr::VaArg { .. }
            | Expr::Error { .. } => None,
        }
    }

    /// Give the constants that `id` decides their values, now that `id` has
    /// its own, and each enumeration whose last value it was its sign.
    ///
    /// A constant's value is the value written for it, or the one before it
    /// plus one, or `0` for the first of its list (C17 6.7.2.2 p3), and none
    /// where the value it counts from has none or `int` does not hold it
    /// (p2), which [`Checker::check_enumerators`] reports. **Stored as each
    /// is decided**, at the expression it counts from: the parser pushed each
    /// written value before any use of the constants it gives, so the arena
    /// order this pass types in reaches it first. Counting back from each use
    /// instead made a list of implicit values quadratic.
    ///
    /// Mutation: count the first of a list from `1`;
    /// `an_enum_gives_each_enumerator_its_value` exits 1.
    fn decide_enumerations(&mut self, ast: &mut Ast, id: Option<ExprId>) {
        let Some(waiting) = self.enumerations_waiting.remove(&id) else {
            return;
        };
        let int = self.target.integer(Rank::Int, Sign::Signed);
        for enumerator in waiting.constants {
            let at = ast.enumerator(enumerator);
            let value = match (at.written, at.previous) {
                (Some(written), _) => self.types[written.index()]
                    .and_then(|ty| integer_of(ast, ty))
                    .and(self.values[written.index()]),
                (None, Some(previous)) => self.enumerator_values[previous.index()].map(|v| v + 1),
                (None, None) => Some(0),
            };
            self.enumerator_values[enumerator.index()] = value.filter(|&value| int.holds(value));
        }
        // p4: the integer type an enumerated type is, once every constant has
        // its value. One with no value leaves `int`, and is reported.
        for definition in waiting.complete {
            let negative = ast
                .enumeration(definition)
                .constants
                .iter()
                .flatten()
                .any(|&e| self.enumerator_values[e.index()].is_some_and(|value| value < 0));
            let sign = self.target.enumeration_sign(negative);
            for &ty in self.enumerated.get(&definition).into_iter().flatten() {
                ast.set_enumeration_sign(ty, sign);
            }
        }
    }

    /// C17 6.7.2.2 p2: the value written for an enumeration constant is an
    /// integer constant expression, and every constant's value is one `int`
    /// holds.
    ///
    /// Reported once per enumerator, at what is wrong: the value written,
    /// where it is not a constant, or the constant's name, where the value it
    /// is given does not fit. A value with a fault already reported inside it
    /// is not reported again.
    ///
    /// Mutation: report nothing here; `an_enumerator_value_c_does_not_allow_is_refused`
    /// accepts both.
    fn check_enumerators(&self, ast: &Ast, diagnostics: &mut DiagnosticSink) {
        for id in ast.enumerator_ids() {
            let enumerator = ast.enumerator(id);
            if let Some(written) = enumerator.written {
                if self.reported_within[written.index()] {
                    continue;
                }
                let integer = self.types[written.index()]
                    .and_then(|ty| integer_of(ast, ty))
                    .is_some();
                if !integer || self.values[written.index()].is_none() {
                    let span = ast.expr(written).span();
                    diagnostics.report(
                        Diagnostic::error("expression is not an integer constant expression")
                            .with_code(NOT_CONSTANT)
                            .with_label(Label::primary(
                                span,
                                "an enumeration constant's value is one",
                            ))
                            .with_note("C17 6.7.2.2 p2"),
                    );
                    continue;
                }
            }
            if self.enumerator_values[id.index()].is_none() {
                let previous_failed = enumerator.written.is_none()
                    && enumerator
                        .previous
                        .is_some_and(|previous| self.enumerator_values[previous.index()].is_none());
                // One that counts on from a constant already reported is not.
                if previous_failed {
                    continue;
                }
                diagnostics.report(
                    Diagnostic::error("enumerator value is not representable in `int`")
                        .with_code(ENUMERATOR_RANGE)
                        .with_label(Label::primary(enumerator.name, "outside `int`"))
                        .with_note("C17 6.7.2.2 p2"),
                );
            }
        }
    }

    /// Whether `id`, an operand C does not evaluate, is what a constant
    /// expression may hold there: constant in shape, as `known` asks, or
    /// built of such operands by an operator `evaluate` reads or by a comma,
    /// which C17 6.6 p3 allows "within a subexpression that is not
    /// evaluated". So `0 && (1, 2)` is a constant, and `0 && x` is not,
    /// since 6.6 p6 asks every operand of an integer constant expression to
    /// be a constant whether or not it is evaluated.
    ///
    /// A loop over a stack rather than a recursion, since a comma or an
    /// operator chain is as long as the source makes it.
    fn unevaluated_constant(&self, ast: &Ast, id: ExprId) -> bool {
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if self.values[id.index()].is_some() || self.undefined[id.index()] {
                continue;
            }
            match *ast.expr(id) {
                Expr::Comma { lhs, rhs, .. } | Expr::Binary { lhs, rhs, .. } => {
                    pending.extend([lhs, rhs]);
                }
                Expr::Unary {
                    op: UnOp::Plus | UnOp::Minus | UnOp::BitNot | UnOp::Not,
                    operand,
                    ..
                } => pending.push(operand),
                Expr::Conditional {
                    condition,
                    then,
                    otherwise,
                    ..
                } => pending.extend([condition, then, otherwise]),
                _ => return false,
            }
        }
        true
    }

    /// The type and the value of an integer constant, C17 6.4.4.1.
    ///
    /// Both at once, because `read_number` answers both from one pass over
    /// the spelling and because a constant with no type has no value either.
    ///
    /// The type is the first in 6.4.4.1 p5's list for the constant's suffix
    /// and base that holds its value, on this target: `2147483648` is a
    /// `long` where `long` is 64 bits and a `long long` where it is 32, and
    /// `0xffffffff` an `unsigned int`. A value no type in its list holds is
    /// refused (6.4.4 p2).
    ///
    /// Mutation: take the first type of the list whatever holds the value;
    /// `a_constant_takes_the_first_type_that_holds_it` fails.
    ///
    /// Nothing that reaches a diagnostic below is the file's own text, which
    /// is what every new message owes, since source text is content: the
    /// span is what points at the spelling, and the words are this
    /// compiler's.
    fn constant(
        &mut self,
        ast: &mut Ast,
        id: ExprId,
        span: Span,
        diagnostics: &mut DiagnosticSink,
    ) -> Option<TypeId> {
        // `Reading` is matched exhaustively so that a sixth answer has to be
        // given a report rather than falling into one written for another.
        let reported = match read_number(self.sources.snippet(span)) {
            Reading::Integer {
                value,
                decimal,
                unsigned,
                longs,
            } => {
                use Rank::{Int, Long, LongLong};
                use Sign::{Signed, Unsigned};
                let list: &[(Rank, Sign)] = match (unsigned, longs, decimal) {
                    (false, 0, true) => &[(Int, Signed), (Long, Signed), (LongLong, Signed)],
                    (false, 0, false) => &[
                        (Int, Signed),
                        (Int, Unsigned),
                        (Long, Signed),
                        (Long, Unsigned),
                        (LongLong, Signed),
                        (LongLong, Unsigned),
                    ],
                    (true, 0, _) => &[(Int, Unsigned), (Long, Unsigned), (LongLong, Unsigned)],
                    (false, 1, true) => &[(Long, Signed), (LongLong, Signed)],
                    (false, 1, false) => &[
                        (Long, Signed),
                        (Long, Unsigned),
                        (LongLong, Signed),
                        (LongLong, Unsigned),
                    ],
                    (true, 1, _) => &[(Long, Unsigned), (LongLong, Unsigned)],
                    (false, _, true) => &[(LongLong, Signed)],
                    (false, _, false) => &[(LongLong, Signed), (LongLong, Unsigned)],
                    (true, _, _) => &[(LongLong, Unsigned)],
                };
                // `holds` takes an `i128`, and a value that does not fit one
                // fits no type here either.
                let held = i128::try_from(value).ok().and_then(|value| {
                    list.iter()
                        .find(|&&(rank, sign)| self.target.integer(rank, sign).holds(value))
                        .map(|&integer| (value, integer))
                });
                match held {
                    Some((value, integer)) => {
                        self.values[id.index()] = Some(value);
                        return Some(self.integer_type(ast, integer));
                    }
                    None => TOO_LARGE,
                }
            }
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
    /// 6.2.5 p21: an aggregate, neither arithmetic nor scalar, and so an
    /// operand of no operator that `binary` asks.
    Struct,
}

impl OperandClass {
    /// `None` for an array or a function, which 6.3.2.1 converts before an
    /// operator sees it, so that one arriving here is not answered.
    fn of(ast: &Ast, ty: TypeId) -> Option<Self> {
        match ast.ty(ty) {
            Type::Integer {
                rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
                sign: _,
            } => Some(Self::Arithmetic),
            Type::Pointer(pointee) => Some(Self::Pointer(*pointee)),
            Type::Void => Some(Self::Void),
            // An operand of no operator, as a struct is.
            Type::Struct { .. } | Type::VaList => Some(Self::Struct),
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
fn unsteppable(
    ast: &Ast,
    pointee: TypeId,
    complete: &dyn Fn(TypeId) -> bool,
) -> Option<&'static str> {
    match ast.ty(pointee) {
        Type::Void => Some("`void` has no size"),
        Type::Function { .. } => Some("a function is not an object"),
        Type::Array { length: None, .. } => Some("an array of unknown length has no size"),
        // Complete where the step is written, which `complete` answers.
        Type::Struct { union, .. } => (!complete(pointee)).then_some(if *union {
            "a union that is not complete here has no size"
        } else {
            "a struct that is not complete here has no size"
        }),
        Type::Integer {
            rank: Rank::Char | Rank::Short | Rank::Int | Rank::Long | Rank::LongLong,
            sign: _,
        }
        | Type::Pointer(_)
        | Type::VaList
        | Type::Array { .. } => None,
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
fn pointee_steppable(
    ast: &Ast,
    pointee: TypeId,
    on_left: bool,
    complete: &dyn Fn(TypeId) -> bool,
) -> Result<(), Refused> {
    match unsteppable(ast, pointee, complete) {
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

/// A value no type in its list holds.
const TOO_LARGE: Reported = Reported {
    code: NO_TYPE,
    message: "no integer type can hold this constant",
    label: "this is wider than every type its spelling allows",
    note: "C17 6.4.4.1 p5 gives a constant the first type in a list that holds it, and 6.4.4 p2 \
           requires a constant's value to be in the range of its type",
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
pub(crate) enum Reading {
    /// An integer constant: its value, and what 6.4.4.1 p5's list for it is
    /// chosen by, whether it is decimal, whether its suffix has a `u`, and
    /// how many `l`s.
    Integer {
        value: u128,
        decimal: bool,
        unsigned: bool,
        longs: u8,
    },
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
pub(crate) fn read_number(text: &str) -> Reading {
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

    Reading::Integer {
        value,
        decimal: radix == 10,
        unsigned: suffix.contains(['u', 'U']),
        longs: u8::try_from(suffix.matches(['l', 'L']).count()).expect("a suffix has at most two"),
    }
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
