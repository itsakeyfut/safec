//! What a braced initializer initializes: C17 6.7.9 p17 to p21 worked out
//! against the type it initializes, into the scalars and the aggregates
//! taken whole that its items write, each with its path from the object.
//!
//! **One walk, asked by both its readers**: the type check holds each item
//! to what it initializes (6.5.16.1 by way of p11), and the lowering writes
//! it there. Two walks would be two answers to which member `{1, 2}` puts
//! the `2` in, and the one that drifted would build a program that does not
//! do what the checked one says. See ADR-0137.

use crate::ast::{Ast, ExprId, Initializer, Type, TypeId};
use crate::sema::Resolution;
use safec_ir::source::Span;

/// Whether `value`, of type `of`, initializes an aggregate of `ty` whole
/// rather than its first scalar: a struct of a compatible type (C17 6.7.9
/// p13), or a string literal for an array of character type (p14). One
/// answer for the type check and the lowering, as the walk is.
pub fn initializes_whole(
    ast: &Ast,
    resolution: &Resolution,
    of: Option<TypeId>,
    value: ExprId,
    ty: TypeId,
) -> bool {
    match ast.ty(ty) {
        Type::Struct { .. } => of.is_some_and(|of| {
            matches!(ast.ty(of), Type::Struct { .. })
                && ast.compatible_unqualified(of, ty, &|a, b| resolution.same_tag(a, b))
        }),
        Type::Array { element, .. } => {
            matches!(ast.expr(value), crate::ast::Expr::String { .. })
                && matches!(
                    ast.ty(ast.unqualified(*element)),
                    Type::Integer {
                        rank: safec_ir::ir::Rank::Char,
                        ..
                    }
                )
        }
        _ => false,
    }
}

/// One step from an object to a subobject of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The member at this index among its struct's members in the order
    /// written, as `types::Member::path` counts them.
    Member(usize),
    /// The element at this index.
    Element(u64),
}

/// What one item of an initializer writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    /// The subobject, from the object initialized.
    pub path: Vec<Step>,
    /// The subobject's type, as its declaration wrote it.
    pub ty: TypeId,
    /// What it is initialized with.
    pub value: ExprId,
    /// Whether the subobject is an aggregate the item initializes whole: a
    /// struct from an expression of its type (p13), or an array of
    /// character type from a string literal (p14). Otherwise it is a scalar.
    pub whole: bool,
}

/// Why an initializer could not be worked out, each a span to point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// More items than the object has subobjects to take them, which p2
    /// forbids.
    TooMany(Span),
    /// `{}`, which C17's grammar has no form of, and which C23 gives a
    /// meaning this compiler does not build yet.
    Empty(Span),
    /// A subobject whose element count this stage cannot say: an array of
    /// unknown size the parser could not complete (`Parser::completed`), or
    /// one of a struct this place cannot see the definition of.
    Unsized(Span),
}

/// What an object is made of, for initializing it.
enum Kind {
    /// No subobjects: one item initializes it.
    Scalar,
    /// `count` elements of `element`.
    Array { element: TypeId, count: u64 },
    /// The members an initializer reaches, in order, each with its index
    /// among the struct's members: every named member and every anonymous
    /// one, but no unnamed bit-field (p9) and no flexible array member
    /// (6.7.2.1 p18); for a union, only the first of them (p17).
    Struct(Vec<(usize, TypeId)>),
}

/// The walk, with what it reads of the program.
#[allow(missing_debug_implementations)] // Two closures, which have no `Debug`.
pub struct Shapes<'a> {
    /// The tree.
    pub ast: &'a Ast,
    /// Where each struct is defined.
    pub resolution: &'a Resolution,
    /// The value of an array's length, where it is a constant.
    pub length: &'a dyn Fn(ExprId) -> Option<u64>,
    /// Whether the expression initializes an aggregate of this type whole:
    /// a struct of a compatible type, or a string literal for an array of
    /// character type.
    pub whole: &'a dyn Fn(ExprId, TypeId) -> bool,
}

impl Shapes<'_> {
    /// The leaves `init` writes in an object of `ty`, in the order written.
    ///
    /// Mutation: in `Shapes::fill`, take an expression item for an
    /// aggregate subobject whole rather than its first scalar; the run of
    /// `a_braced_initializer_fills_arrays_and_structs`, whose lists elide
    /// braces, fails.
    pub fn resolve(&self, ty: TypeId, init: &Initializer) -> Result<Vec<Leaf>, Fault> {
        let mut leaves = Vec::new();
        self.object(ty, init, &mut Vec::new(), &mut leaves)?;
        Ok(leaves)
    }

    /// Every scalar of an object of `ty`, in order, with its path and type:
    /// what an initializer that does not name them all sets to zero
    /// (p21). `None` past `limit` of them, or where one's count is unknown.
    pub fn scalars(&self, ty: TypeId, limit: usize) -> Option<Vec<(Vec<Step>, TypeId)>> {
        // **Counted first, by arithmetic over the type**, so that the walk
        // below never visits more than `limit` scalars, and skips an array
        // of elements with none: a struct of a billion empty ones walked
        // element by element took minutes (#1241 review).
        if self.count(ty)? > limit as u64 {
            return None;
        }
        let mut found = Vec::new();
        let mut work = vec![(Vec::new(), ty)];
        while let Some((path, ty)) = work.pop() {
            match self.kind(ty)? {
                Kind::Scalar => {
                    found.push((path, ty));
                    if found.len() > limit {
                        return None;
                    }
                }
                Kind::Array { element, count } => {
                    if self.count(element) == Some(0) {
                        continue;
                    }
                    for index in (0..count).rev() {
                        let mut inner = path.clone();
                        inner.push(Step::Element(index));
                        work.push((inner, element));
                    }
                }
                Kind::Struct(members) => {
                    for &(index, member) in members.iter().rev() {
                        let mut inner = path.clone();
                        inner.push(Step::Member(index));
                        work.push((inner, member));
                    }
                }
            }
        }
        Some(found)
    }

    /// How many scalars an object of `ty` holds, a union counting its first
    /// member's; `None` where that is unknown or past `u64`.
    pub fn count(&self, ty: TypeId) -> Option<u64> {
        match self.kind(ty)? {
            Kind::Scalar => Some(1),
            Kind::Array { element, count } => self.count(element)?.checked_mul(count),
            Kind::Struct(members) => members.iter().try_fold(0u64, |sum, &(_, member)| {
                sum.checked_add(self.count(member)?)
            }),
        }
    }

    /// The type of the subobject `step` reaches in an object of `ty`.
    pub fn subobject(&self, ty: TypeId, step: Step) -> Option<TypeId> {
        match (self.kind(ty)?, step) {
            (Kind::Array { element, .. }, Step::Element(_)) => Some(element),
            (Kind::Struct(members), Step::Member(index)) => members
                .iter()
                .find(|&&(at, _)| at == index)
                .map(|&(_, member)| member),
            _ => None,
        }
    }

    /// Whether an object of `ty` holds a union, anywhere in it, whose first
    /// member is smaller than the union, by `size`: one an initializer
    /// writing its first member leaves bytes of unwritten.
    pub fn holds_short_union(
        &self,
        ty: TypeId,
        size: &mut dyn FnMut(TypeId) -> Option<u64>,
    ) -> bool {
        let mut work = vec![ty];
        let mut seen = 0usize;
        while let Some(ty) = work.pop() {
            // Each type once per path is enough to answer; the bound keeps
            // a pathological nesting from running long.
            seen += 1;
            if seen > 1 << 16 {
                return true;
            }
            match self.ast.ty(ty) {
                Type::Array { element, .. } => work.push(*element),
                Type::Struct { .. } => {
                    let Some((union, members)) = self.members(ty) else {
                        continue;
                    };
                    if union
                        && members
                            .first()
                            .is_some_and(|&(_, first)| size(first) != size(ty))
                    {
                        return true;
                    }
                    work.extend(members.iter().map(|&(_, member)| member));
                }
                _ => {}
            }
        }
        false
    }

    /// Whether `ty` is a union.
    pub fn union(&self, ty: TypeId) -> bool {
        matches!(self.ast.ty(ty), Type::Struct { union: true, .. })
    }

    /// The object of `ty` at `path`, initialized from `init`.
    fn object(
        &self,
        ty: TypeId,
        init: &Initializer,
        path: &mut Vec<Step>,
        leaves: &mut Vec<Leaf>,
    ) -> Result<(), Fault> {
        let (items, span) = match init {
            Initializer::Expr(value) => {
                leaves.push(Leaf {
                    path: path.clone(),
                    ty,
                    value: *value,
                    whole: !matches!(self.kind(ty), Some(Kind::Scalar)),
                });
                return Ok(());
            }
            Initializer::List { items, span } => (items, *span),
        };
        match self.kind(ty).ok_or(Fault::Unsized(span))? {
            // p11: a scalar's initializer is one expression, optionally
            // in braces.
            Kind::Scalar => match items.as_slice() {
                [] => Err(Fault::Empty(span)),
                [one] => self.object(ty, one, path, leaves),
                [_, extra, ..] => Err(Fault::TooMany(extra.span(self.ast))),
            },
            Kind::Array { .. } | Kind::Struct(_) => {
                if items.is_empty() {
                    return Err(Fault::Empty(span));
                }
                // p14: a string literal for an array of character type,
                // "optionally enclosed in braces", at any depth, so
                // `{{"ab"}, {"c"}}` gives each row its literal (#1241
                // review). The parser drops the braces of a declarator's
                // own, so its length counts the literal.
                if let [Initializer::Expr(value)] = items.as_slice() {
                    if (self.whole)(*value, ty)
                        && matches!(self.ast.expr(*value), crate::ast::Expr::String { .. })
                    {
                        leaves.push(Leaf {
                            path: path.clone(),
                            ty,
                            value: *value,
                            whole: true,
                        });
                        return Ok(());
                    }
                }
                let mut at = 0;
                self.fill(ty, items, &mut at, path, leaves)?;
                match items.get(at) {
                    Some(extra) => Err(Fault::TooMany(extra.span(self.ast))),
                    None => Ok(()),
                }
            }
        }
    }

    /// The subobjects of the aggregate of `ty` at `path`, in order, from
    /// `items` from `at` on, until either runs out (p17). A list item
    /// initializes the next subobject whole; an expression initializes it
    /// where it is a scalar or the expression takes it whole, and its first
    /// scalar otherwise, the rest of its subobjects taking the items after
    /// it (p20). A recursion, bounded by how deep the type nests.
    fn fill(
        &self,
        ty: TypeId,
        items: &[Initializer],
        at: &mut usize,
        path: &mut Vec<Step>,
        leaves: &mut Vec<Leaf>,
    ) -> Result<(), Fault> {
        let span = |at: usize| items[at.min(items.len() - 1)].span(self.ast);
        let subobjects: Vec<(Step, TypeId)> =
            match self.kind(ty).ok_or(Fault::Unsized(span(*at)))? {
                Kind::Scalar => unreachable!("only an aggregate is filled"),
                Kind::Array { element, count } => {
                    // No more elements than items are left: a long array takes
                    // what is written and leaves the rest to zero.
                    let left = (items.len() - *at) as u64;
                    (0..count.min(left))
                        .map(|index| (Step::Element(index), element))
                        .collect()
                }
                Kind::Struct(members) => members
                    .into_iter()
                    .map(|(index, member)| (Step::Member(index), member))
                    .collect(),
            };
        for (step, sub) in subobjects {
            if *at == items.len() {
                break;
            }
            path.push(step);
            self.one(sub, items, at, path, leaves)?;
            path.pop();
        }
        Ok(())
    }

    /// The one subobject of `sub` at `path`, from `items` from `at` on: a
    /// list item initializes it whole; an expression initializes it where
    /// it is a scalar or the expression takes it whole, and its first
    /// scalar otherwise, the rest of its subobjects taking the items after
    /// it (p20).
    fn one(
        &self,
        sub: TypeId,
        items: &[Initializer],
        at: &mut usize,
        path: &mut Vec<Step>,
        leaves: &mut Vec<Leaf>,
    ) -> Result<(), Fault> {
        let item = &items[*at];
        match item {
            Initializer::List { .. } => {
                *at += 1;
                self.object(sub, item, path, leaves)
            }
            Initializer::Expr(value) => {
                let scalar = matches!(
                    self.kind(sub).ok_or(Fault::Unsized(item.span(self.ast)))?,
                    Kind::Scalar
                );
                if scalar || (self.whole)(*value, sub) {
                    *at += 1;
                    leaves.push(Leaf {
                        path: path.clone(),
                        ty: sub,
                        value: *value,
                        whole: !scalar,
                    });
                    Ok(())
                } else {
                    self.fill(sub, items, at, path, leaves)
                }
            }
        }
    }

    /// How many elements of `element` the list `init` initializes, which is
    /// the length of an array of unknown size it is the initializer of (C17
    /// 6.7.9 p22): as many as `Shapes::fill` would take of an array long
    /// enough, brace elision included. Asked by the type check once every
    /// item is typed, since a struct taken whole is one item where its
    /// members elided would be several. See `Parser::completed`.
    ///
    /// Mutation: count every item as an element; the run of
    /// `a_braced_initializer_fills_arrays_and_structs` sizes `elided`
    /// wrongly.
    pub fn extent(&self, element: TypeId, init: &Initializer) -> Result<u64, Fault> {
        let Initializer::List { items, span } = init else {
            return Err(Fault::Unsized(init.span(self.ast)));
        };
        if items.is_empty() {
            return Err(Fault::Empty(*span));
        }
        let (mut at, mut count) = (0, 0u64);
        let mut scratch = Vec::new();
        while at < items.len() {
            let before = at;
            self.one(element, items, &mut at, &mut Vec::new(), &mut scratch)?;
            scratch.clear();
            // An element that takes no item, a struct with no member an
            // initializer reaches, would count forever.
            if at == before {
                return Err(Fault::TooMany(items[at].span(self.ast)));
            }
            count += 1;
        }
        Ok(count)
    }

    /// Whether the struct `ty` is a union, and every member an initializer
    /// can reach, each with its index among the members written: every
    /// named member and every anonymous one, but no unnamed bit-field (p9)
    /// and no flexible array member (6.7.2.1 p18). `None` where its
    /// definition is not to be had.
    fn members(&self, ty: TypeId) -> Option<(bool, Vec<(usize, TypeId)>)> {
        let Type::Struct { union, members, .. } = self.ast.ty(ty) else {
            return None;
        };
        let members = match members {
            Some(members) => members,
            None => {
                let bound = self.resolution.bound(self.ast, ty);
                let definition = self.resolution.definition(self.resolution.tag(bound))?;
                match self.ast.ty(definition) {
                    Type::Struct {
                        members: Some(members),
                        ..
                    } => members,
                    _ => return None,
                }
            }
        };
        let last = members.len().checked_sub(1);
        let reached = members
            .iter()
            .enumerate()
            .filter(|&(index, member)| {
                let unnamed_bits = member.name.is_none() && member.width.is_some();
                let flexible = Some(index) == last
                    && matches!(self.ast.ty(member.ty), Type::Array { length: None, .. });
                !unnamed_bits && !flexible
            })
            .map(|(index, member)| (index, member.ty))
            .collect();
        Some((*union, reached))
    }

    /// What an object of `ty` is made of, or `None` where an array's count
    /// is unknown or a struct's definition is not to be had.
    fn kind(&self, ty: TypeId) -> Option<Kind> {
        match self.ast.ty(ty) {
            Type::Array {
                element, length, ..
            } => Some(Kind::Array {
                element: *element,
                count: (self.length)((*length)?)?,
            }),
            Type::Struct { .. } => {
                let (union, mut reached) = self.members(ty)?;
                if union {
                    reached.truncate(1);
                }
                Some(Kind::Struct(reached))
            }
            Type::Void
            | Type::VaList
            | Type::Integer { .. }
            | Type::Pointer(_)
            | Type::Function { .. } => Some(Kind::Scalar),
        }
    }
}
